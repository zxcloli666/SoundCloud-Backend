from __future__ import annotations

import argparse
import fnmatch
import functools
import hashlib
import http.client
import importlib.util
import os
import shutil
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import BinaryIO

import httpx
import orjson
from huggingface_hub import HfApi, constants, hf_hub_download, snapshot_download
from huggingface_hub.errors import EntryNotFoundError
from huggingface_hub.file_download import repo_folder_name

from worker import settings as settings_module
from worker.settings import Settings, SettingsError, TtsFile, TtsSection, TtsVoice

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_CONFIG = 78
DEFAULT_CONFIG_DIR = Path("config")
FASTTEXT_HOME_ENV = "FASTTEXT_HOME"
ROFORMER_HOME_ENV = "MELBAND_ROFORMER_MODELS_PATH"
TTS_HOME_ENV = "SCD_TTS_MODELS_PATH"
EXECUTABLE_MODE = 0o755
FASTTEXT_SLOT = "cpu-tools"
ROFORMER_SLOT = "sep"
MAIN_BRANCH = "main"
DOWNLOAD_TIMEOUT_S = 60
CHUNK_BYTES = 1 << 20
MIB = 1 << 20
THROTTLE_BURST_S = 1.0
THROTTLE_MIN_PAUSE_S = 0.05
RETRY_PAUSES_S = (2.0, 5.0, 15.0, 30.0, 60.0)
PARTIAL_SUFFIX = ".partial"
HTTP_PARTIAL_CONTENT = 206
HTTP_RANGE_NOT_SATISFIABLE = 416
HTTP_CLIENT_ERRORS = range(400, 500)
HTTP_RETRIED_CLIENT_ERRORS = frozenset({408, 425, 429})
DOWNLOAD_ERRORS = (OSError, http.client.HTTPException)
SKIPPED_FILES = (
    "*.md",
    ".gitattributes",
    "LICENSE*",
    "*.h5",
    "*.msgpack",
    "*.onnx",
    "*.ot",
    "*.tflite",
    "onnx/*",
    "openvino/*",
    "coreml/*",
)
PICKLED_WEIGHTS = ("*.bin", "*.pt", "*.pth")


class FetchError(RuntimeError):
    pass


class Throttle:
    def __init__(
        self,
        bytes_per_s: float,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.bytes_per_s = bytes_per_s
        self._clock = clock
        self._sleep = sleep
        self._due: float | None = None

    @classmethod
    def of(cls, mib_per_s: float) -> Throttle:
        return cls(mib_per_s * MIB)

    def passed(self, size: int) -> None:
        if self.bytes_per_s <= 0:
            return
        now = self._clock()
        earliest = now - THROTTLE_BURST_S
        start = earliest if self._due is None else max(self._due, earliest)
        self._due = start + size / self.bytes_per_s
        pause = self._due - now
        if pause >= THROTTLE_MIN_PAUSE_S:
            self._sleep(pause)


UNLIMITED = Throttle(0.0)


@dataclass(frozen=True)
class HubSnapshot:
    repo: str
    revision: str
    loaded_by_name: bool = False

    def describe(self) -> str:
        return f"hf:{self.repo}@{self.revision}"


@dataclass(frozen=True)
class HubFile:
    repo: str
    revision: str
    filename: str
    sha256: str

    def describe(self) -> str:
        return f"hf:{self.repo}@{self.revision}/{self.filename}"


@dataclass(frozen=True)
class VerifiedDownload:
    urls: tuple[str, ...]
    sha256: str
    target: Path
    executable: bool = False
    throttled: bool = False

    def describe(self) -> str:
        return self.urls[0]


@dataclass(frozen=True)
class PackagedFile:
    package: str
    relative: str
    sha256: str
    target: Path

    def describe(self) -> str:
        return f"package:{self.package}/{self.relative}"


Artifact = HubSnapshot | HubFile | VerifiedDownload | PackagedFile


@dataclass(frozen=True)
class RoformerSource:
    repo: str
    commit: str
    checkpoint: str
    config: str
    config_sha256: str

    def checkpoint_url(self) -> str:
        return f"https://huggingface.co/{self.repo}/resolve/{self.commit}/{self.checkpoint}"


@dataclass(frozen=True)
class Item:
    slot: str
    artifact: Artifact


COMPANIONS: Mapping[str, tuple[HubSnapshot, ...]] = MappingProxyType(
    {
        "OpenMuQ/MuQ-MuLan-large": (
            HubSnapshot(
                "OpenMuQ/MuQ-large-msd-iter",
                "0562a57814f6f8bbd9fdea0a25921a2fce1a841a",
                loaded_by_name=True,
            ),
            HubSnapshot(
                "xlm-roberta-base",
                "e73636d4f797dec63c3081bb6ed5c7b0bb3f2089",
                loaded_by_name=True,
            ),
        ),
    }
)
ROFORMER_SOURCES: Mapping[str, RoformerSource] = MappingProxyType(
    {
        "melband-roformer-kim-vocals": RoformerSource(
            repo="KimberleyJSN/melbandroformer",
            commit="ac9b0614ab3cd7f77219e18ba494dfd93956c348",
            checkpoint="MelBandRoformer.ckpt",
            config="config_vocals_mel_band_roformer.yaml",
            config_sha256="5e380dfa5d5757ac4c2b7f6ef607b93d5058ecff805e7b05ed730a47b90d103c",
        ),
    }
)
ROFORMER_PACKAGE = "mel_band_roformer"
FETCH_ERRORS = (OSError, httpx.HTTPError, EntryNotFoundError, FetchError)


def main(argv: Sequence[str], environ: Mapping[str, str] = os.environ) -> int:
    args = parse_args(argv)
    env = dict(environ)
    if args.profile:
        env[settings_module.PROFILE_ENV] = args.profile
    try:
        require_online()
        settings = settings_module.load(args.config_dir, env)
        items = plan(settings, env)
    except (SettingsError, FetchError) as error:
        report("fetch_models_config_error", error=str(error))
        return EXIT_CONFIG
    throttle = Throttle.of(settings.tts.download_mib_s)
    failures = fetch_all(items, functools.partial(fetch, throttle=throttle))
    report(
        "fetch_models_finished",
        profile=settings.profile,
        fetched=len(items) - failures,
        failed=failures,
    )
    return EXIT_FAILED if failures else EXIT_OK


def plan(settings: Settings, environ: Mapping[str, str]) -> list[Item]:
    items: list[Item] = []
    seen: set[Artifact] = set()
    for slot in settings_module.slots_for_lanes(settings):
        for artifact in artifacts_for(slot, settings, environ):
            if artifact not in seen:
                seen.add(artifact)
                items.append(Item(slot, artifact))
    return items


def artifacts_for(slot: str, settings: Settings, environ: Mapping[str, str]) -> list[Artifact]:
    if slot == settings_module.TTS_SLOT:
        return tts_artifacts(settings.tts, home(environ, TTS_HOME_ENV))
    spec = settings.slots[slot]
    if not spec.model:
        return []
    if slot == settings_module.LOCAL_LLM_SLOT:
        local = settings.llm.local
        return [HubFile(spec.model, spec.revision, local.file, local.sha256)]
    if slot == FASTTEXT_SLOT:
        target = home(environ, FASTTEXT_HOME_ENV) / spec.model.rsplit("/", 1)[-1]
        return [VerifiedDownload((spec.model,), spec.revision, target)]
    if slot == ROFORMER_SLOT:
        return roformer_artifacts(spec, home(environ, ROFORMER_HOME_ENV))
    return [HubSnapshot(spec.model, spec.revision), *COMPANIONS.get(spec.model, ())]


def roformer_artifacts(spec: settings_module.SlotSettings, models_dir: Path) -> list[Artifact]:
    source = ROFORMER_SOURCES.get(spec.model)
    if source is None:
        raise FetchError(f"slots.{ROFORMER_SLOT}.model {spec.model!r} has no known source")
    directory = models_dir / spec.model
    return [
        VerifiedDownload((source.checkpoint_url(),), spec.revision, directory / source.checkpoint),
        PackagedFile(
            ROFORMER_PACKAGE,
            f"configs/{source.config}",
            source.config_sha256,
            directory / source.config,
        ),
    ]


def tts_artifacts(tts: TtsSection, models_dir: Path) -> list[Artifact]:
    engine = tts.engine
    programs: list[Artifact] = [
        VerifiedDownload(
            mirrored(tts, engine.path(name)),
            sha256,
            models_dir / engine.path(name),
            executable=name == engine.program,
            throttled=True,
        )
        for name, sha256 in engine.files_for(tts.device).items()
    ]
    weights: list[TtsFile | TtsVoice] = [tts.codec, *tts.voices.values()]
    models: list[Artifact] = [
        VerifiedDownload(
            mirrored(tts, model.path), model.sha256, models_dir / model.path, throttled=True
        )
        for model in weights
    ]
    return programs + models


def mirrored(tts: TtsSection, path: str) -> tuple[str, ...]:
    return tuple(f"{base}/{path}" for base in tts.base_urls)


def fetch_all(items: Sequence[Item], fetcher: Callable[[Artifact], None]) -> int:
    failures = 0
    for item in items:
        source = item.artifact.describe()
        try:
            fetcher(item.artifact)
        except FETCH_ERRORS as error:
            failures += 1
            report("model_fetch_failed", slot=item.slot, source=source, error=repr(error))
            continue
        report("model_fetched", slot=item.slot, source=source)
    return failures


def fetch(artifact: Artifact, throttle: Throttle = UNLIMITED) -> None:
    match artifact:
        case HubSnapshot():
            fetch_snapshot(artifact, HfApi())
        case HubFile():
            fetch_file(artifact)
        case VerifiedDownload():
            fetch_download(artifact, throttle if artifact.throttled else UNLIMITED)
        case PackagedFile():
            fetch_packaged(artifact)


def fetch_snapshot(snapshot: HubSnapshot, api: HfApi) -> None:
    repo, requested = snapshot.repo, snapshot.revision
    if snapshot.loaded_by_name:
        info = api.model_info(snapshot.repo, revision=MAIN_BRANCH)
        if info.sha != snapshot.revision:
            raise FetchError(f"{snapshot.repo}: main is at {info.sha}, pinned {snapshot.revision}")
        repo, requested = info.id, MAIN_BRANCH
    files = api.list_repo_files(repo, revision=snapshot.revision)
    path = Path(snapshot_download(repo, revision=requested, allow_patterns=chosen_files(files)))
    if path.name != snapshot.revision:
        raise FetchError(f"{snapshot.repo}: got snapshot {path.name}, pinned {snapshot.revision}")
    if repo != snapshot.repo:
        link_renamed_snapshot(path, snapshot)


def fetch_file(file: HubFile) -> None:
    path = Path(hf_hub_download(file.repo, file.filename, revision=file.revision))
    if file.revision not in path.parts:
        raise FetchError(f"{file.describe()}: got {path}, not the pinned commit")
    if has_sha256(path, file.sha256):
        return
    remove_with_links(path)
    raise FetchError(f"{file.describe()}: sha256 != {file.sha256}")


def remove_with_links(path: Path) -> None:
    while path.is_symlink():
        target = Path(os.path.normpath(path.parent / os.readlink(path)))
        path.unlink()
        path = target
    path.unlink(missing_ok=True)


def link_renamed_snapshot(canonical: Path, snapshot: HubSnapshot) -> None:
    legacy = canonical.parents[2] / repo_folder_name(repo_id=snapshot.repo, repo_type="model")
    target_dir = legacy / "snapshots" / snapshot.revision
    for source in canonical.rglob("*"):
        if source.is_dir():
            continue
        link = target_dir / source.relative_to(canonical)
        if link.exists():
            continue
        link.parent.mkdir(parents=True, exist_ok=True)
        link.unlink(missing_ok=True)
        link.symlink_to(os.path.relpath(source.resolve(), link.parent))
    refs = legacy / "refs"
    refs.mkdir(parents=True, exist_ok=True)
    (refs / MAIN_BRANCH).write_text(snapshot.revision)


def chosen_files(files: Sequence[str]) -> list[str]:
    kept = [name for name in files if not matches_any(name, SKIPPED_FILES)]
    if any(name.endswith(".safetensors") for name in kept):
        kept = [name for name in kept if not matches_any(name, PICKLED_WEIGHTS)]
    return kept


def fetch_download(download: VerifiedDownload, throttle: Throttle = UNLIMITED) -> None:
    if not has_sha256(download.target, download.sha256):
        replace_verified(download, throttle)
    if download.executable:
        download.target.chmod(EXECUTABLE_MODE)


def replace_verified(download: VerifiedDownload, throttle: Throttle) -> None:
    download.target.parent.mkdir(parents=True, exist_ok=True)
    partial = download.target.with_name(download.target.name + PARTIAL_SUFFIX)
    failures: list[str] = []
    for url in download.urls:
        try:
            verified_from(url, download.sha256, partial, throttle)
        except FetchError as error:
            failures.append(str(error))
            report("model_address_failed", url=url, error=str(error))
            continue
        partial.replace(download.target)
        return
    raise FetchError("; ".join(failures))


def verified_from(url: str, sha256: str, partial: Path, throttle: Throttle) -> None:
    resumed = download_with_retries(url, partial, throttle)
    digest = file_sha256(partial)
    if digest != sha256 and resumed:
        partial.unlink()
        report("model_download_restarted", url=url, why="sha256 after resume")
        download_with_retries(url, partial, throttle)
        digest = file_sha256(partial)
    if digest != sha256:
        partial.unlink()
        raise FetchError(f"{url}: sha256 {digest} != {sha256}")


def download_with_retries(url: str, partial: Path, throttle: Throttle) -> bool:
    pauses = iter(RETRY_PAUSES_S)
    attempts = 0
    while True:
        attempts += 1
        try:
            return resume_to(url, partial, throttle)
        except DOWNLOAD_ERRORS as error:
            pause = None if refused(error) else next(pauses, None)
            if pause is None:
                raise FetchError(f"{url}: {error!r} (attempts: {attempts})") from error
            report(
                "model_download_retry",
                url=url,
                attempt=attempts,
                kept_bytes=size_of(partial),
                pause_s=pause,
                error=repr(error),
            )
            time.sleep(pause)


def refused(error: Exception) -> bool:
    return (
        isinstance(error, urllib.error.HTTPError)
        and error.code in HTTP_CLIENT_ERRORS
        and error.code not in HTTP_RETRIED_CLIENT_ERRORS
    )


def resume_to(url: str, partial: Path, throttle: Throttle) -> bool:
    kept = size_of(partial)
    try:
        response = urllib.request.urlopen(ranged(url, kept), timeout=DOWNLOAD_TIMEOUT_S)
    except urllib.error.HTTPError as error:
        if kept and error.code == HTTP_RANGE_NOT_SATISFIABLE:
            return True
        raise
    with response:
        resumed = bool(kept) and response.status == HTTP_PARTIAL_CONTENT
        with partial.open("ab" if resumed else "wb") as out:
            written = copy_body(response, out, throttle)
        promised = response.headers.get("Content-Length")
    if promised is not None and written < int(promised):
        raise http.client.IncompleteRead(b"", int(promised) - written)
    return resumed


def ranged(url: str, kept: int) -> urllib.request.Request:
    headers = {"Range": f"bytes={kept}-"} if kept else {}
    return urllib.request.Request(url, headers=headers)


def copy_body(response: BinaryIO, out: BinaryIO, throttle: Throttle) -> int:
    written = 0
    while chunk := response.read(CHUNK_BYTES):
        written += out.write(chunk)
        throttle.passed(len(chunk))
    return written


def size_of(path: Path) -> int:
    return path.stat().st_size if path.is_file() else 0


def fetch_packaged(packaged: PackagedFile) -> None:
    if has_sha256(packaged.target, packaged.sha256):
        return
    source = package_dir(packaged.package) / packaged.relative
    if not has_sha256(source, packaged.sha256):
        raise FetchError(f"{source}: missing or sha256 != {packaged.sha256}")
    packaged.target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, packaged.target)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="worker fetch-models")
    parser.add_argument("--profile", default="")
    parser.add_argument("--config-dir", type=Path, default=DEFAULT_CONFIG_DIR)
    return parser.parse_args(list(argv))


def require_online() -> None:
    if constants.is_offline_mode():
        raise FetchError("HF_HUB_OFFLINE is set: run fetch-models with HF_HUB_OFFLINE=0")


def home(environ: Mapping[str, str], name: str) -> Path:
    value = environ.get(name, "")
    if not value:
        raise FetchError(f"{name} is not set")
    return Path(value)


def package_dir(package: str) -> Path:
    spec = importlib.util.find_spec(package)
    if spec is None or not spec.submodule_search_locations:
        raise FetchError(f"package {package} is not installed")
    return Path(next(iter(spec.submodule_search_locations)))


def has_sha256(path: Path, expected: str) -> bool:
    return path.is_file() and file_sha256(path) == expected


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest()


def matches_any(name: str, patterns: Sequence[str]) -> bool:
    return any(fnmatch.fnmatch(name, pattern) for pattern in patterns)


def report(event: str, **fields: object) -> None:
    sys.stdout.write(orjson.dumps({"event": event, **fields}).decode() + "\n")
    sys.stdout.flush()
