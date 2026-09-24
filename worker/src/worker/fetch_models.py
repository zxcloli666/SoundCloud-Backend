from __future__ import annotations

import argparse
import fnmatch
import hashlib
import importlib.util
import os
import shutil
import sys
import urllib.request
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType

import httpx
import orjson
from huggingface_hub import HfApi, constants, snapshot_download
from huggingface_hub.errors import EntryNotFoundError

from worker import settings as settings_module
from worker.settings import Settings, SettingsError

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_CONFIG = 78
DEFAULT_CONFIG_DIR = Path("config")
FASTTEXT_HOME_ENV = "FASTTEXT_HOME"
ROFORMER_HOME_ENV = "MELBAND_ROFORMER_MODELS_PATH"
FASTTEXT_SLOT = "cpu-tools"
ROFORMER_SLOT = "sep"
MAIN_BRANCH = "main"
DOWNLOAD_TIMEOUT_S = 60
CHUNK_BYTES = 1 << 20
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


@dataclass(frozen=True)
class HubSnapshot:
    repo: str
    revision: str
    loaded_by_name: bool = False

    def describe(self) -> str:
        return f"hf:{self.repo}@{self.revision}"


@dataclass(frozen=True)
class VerifiedDownload:
    url: str
    sha256: str
    target: Path

    def describe(self) -> str:
        return self.url


@dataclass(frozen=True)
class PackagedFile:
    package: str
    relative: str
    sha256: str
    target: Path

    def describe(self) -> str:
        return f"package:{self.package}/{self.relative}"


Artifact = HubSnapshot | VerifiedDownload | PackagedFile


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
    failures = fetch_all(items, fetch)
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
        for artifact in artifacts_for(slot, settings.slots[slot], environ):
            if artifact not in seen:
                seen.add(artifact)
                items.append(Item(slot, artifact))
    return items


def artifacts_for(
    slot: str, spec: settings_module.SlotSettings, environ: Mapping[str, str]
) -> list[Artifact]:
    if not spec.model:
        return []
    if slot == FASTTEXT_SLOT:
        target = home(environ, FASTTEXT_HOME_ENV) / spec.model.rsplit("/", 1)[-1]
        return [VerifiedDownload(spec.model, spec.revision, target)]
    if slot == ROFORMER_SLOT:
        return roformer_artifacts(spec, home(environ, ROFORMER_HOME_ENV))
    return [HubSnapshot(spec.model, spec.revision), *COMPANIONS.get(spec.model, ())]


def roformer_artifacts(spec: settings_module.SlotSettings, models_dir: Path) -> list[Artifact]:
    source = ROFORMER_SOURCES.get(spec.model)
    if source is None:
        raise FetchError(f"slots.{ROFORMER_SLOT}.model {spec.model!r} has no known source")
    directory = models_dir / spec.model
    return [
        VerifiedDownload(source.checkpoint_url(), spec.revision, directory / source.checkpoint),
        PackagedFile(
            ROFORMER_PACKAGE,
            f"configs/{source.config}",
            source.config_sha256,
            directory / source.config,
        ),
    ]


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


def fetch(artifact: Artifact) -> None:
    match artifact:
        case HubSnapshot():
            fetch_snapshot(artifact, HfApi())
        case VerifiedDownload():
            fetch_download(artifact)
        case PackagedFile():
            fetch_packaged(artifact)


def fetch_snapshot(snapshot: HubSnapshot, api: HfApi) -> None:
    requested = snapshot.revision
    if snapshot.loaded_by_name:
        requested = MAIN_BRANCH
        head = api.model_info(snapshot.repo, revision=MAIN_BRANCH).sha
        if head != snapshot.revision:
            raise FetchError(f"{snapshot.repo}: main is at {head}, pinned {snapshot.revision}")
    files = api.list_repo_files(snapshot.repo, revision=snapshot.revision)
    path = Path(
        snapshot_download(snapshot.repo, revision=requested, allow_patterns=chosen_files(files))
    )
    if path.name != snapshot.revision:
        raise FetchError(f"{snapshot.repo}: got snapshot {path.name}, pinned {snapshot.revision}")


def chosen_files(files: Sequence[str]) -> list[str]:
    kept = [name for name in files if not matches_any(name, SKIPPED_FILES)]
    if any(name.endswith(".safetensors") for name in kept):
        kept = [name for name in kept if not matches_any(name, PICKLED_WEIGHTS)]
    return kept


def fetch_download(download: VerifiedDownload) -> None:
    if has_sha256(download.target, download.sha256):
        return
    download.target.parent.mkdir(parents=True, exist_ok=True)
    partial = download.target.with_name(download.target.name + ".partial")
    try:
        digest = stream_to(download.url, partial)
    except OSError:
        partial.unlink(missing_ok=True)
        raise
    if digest != download.sha256:
        partial.unlink()
        raise FetchError(f"{download.url}: sha256 {digest} != {download.sha256}")
    partial.replace(download.target)


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


def stream_to(url: str, into: Path) -> str:
    digest = hashlib.sha256()
    with (
        urllib.request.urlopen(url, timeout=DOWNLOAD_TIMEOUT_S) as response,
        into.open("wb") as out,
    ):
        while chunk := response.read(CHUNK_BYTES):
            digest.update(chunk)
            out.write(chunk)
    return digest.hexdigest()


def has_sha256(path: Path, expected: str) -> bool:
    if not path.is_file():
        return False
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(CHUNK_BYTES):
            digest.update(chunk)
    return digest.hexdigest() == expected


def matches_any(name: str, patterns: Sequence[str]) -> bool:
    return any(fnmatch.fnmatch(name, pattern) for pattern in patterns)


def report(event: str, **fields: object) -> None:
    sys.stdout.write(orjson.dumps({"event": event, **fields}).decode() + "\n")
    sys.stdout.flush()
