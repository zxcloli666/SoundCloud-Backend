from __future__ import annotations

import ast
import subprocess
import sys
from pathlib import Path

import pytest

SRC = Path(__file__).resolve().parents[2] / "src" / "worker"

ENGINE_PROCESS_MODULES = frozenset(
    {"worker.runtime.engine_main", "worker.runtime.allocator", "worker.runtime.devices"}
)
NATIVE_PACKAGES = (
    "torch",
    "torchaudio",
    "torchvision",
    "torchcodec",
    "transformers",
    "sentence_transformers",
    "muq",
    "silero_vad",
    "mel_band_roformer",
    "ctc_forced_aligner",
    "dynet",
    "fasttext",
    "gensim",
    "chromaprint",
    "bitsandbytes",
)
LOADED_ON_FIRST_USE = ("nagisa",)
BUS_ALLOWED = (
    "worker.bus",
    "worker.contract",
    "worker.settings",
    "worker.domain.outcome",
    "worker.domain.deadline",
    "worker.observability.counters",
)
DOMAIN_FORBIDDEN = ("nats", "worker.bus", "worker.runtime", "worker.app", "worker.llm")
RUNTIME_FORBIDDEN = ("nats", "worker.domain", "worker.bus", "worker.app", "worker.llm")
CONSUMER_CREATING_CALLS = frozenset({"pull_subscribe", "add_consumer"})


def module_files() -> list[Path]:
    return sorted(SRC.rglob("*.py"))


def module_name(path: Path) -> str:
    relative = path.relative_to(SRC.parent).with_suffix("")
    parts = list(relative.parts)
    if parts[-1] == "__init__":
        parts.pop()
    return ".".join(parts)


def imports_of(path: Path) -> set[str]:
    package = module_name(path).split(".")
    if path.name != "__init__.py":
        package.pop()
    return imports_in(path.read_text(encoding="utf-8"), package)


def imports_in(source: str, package: list[str]) -> set[str]:
    names: set[str] = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            base = imported_from(node, package)
            names.update(
                base if alias.name == "*" else f"{base}.{alias.name}" for alias in node.names
            )
    return names


def imported_from(node: ast.ImportFrom, package: list[str]) -> str:
    if node.level == 0:
        return node.module or ""
    anchor = package[: len(package) - (node.level - 1)]
    return ".".join([*anchor, *([node.module] if node.module else [])])


def is_under(name: str, prefixes: tuple[str, ...]) -> bool:
    return any(name == prefix or name.startswith(prefix + ".") for prefix in prefixes)


def is_engine_module(name: str) -> bool:
    return name in ENGINE_PROCESS_MODULES or is_under(name, ("worker.models",))


def attribute_calls(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return {
        node.func.attr
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
    }


@pytest.mark.parametrize(
    ("source", "package", "expected"),
    [
        ("import torch.nn", ["worker", "domain"], {"torch.nn"}),
        ("from worker import bus", ["worker", "domain"], {"worker.bus"}),
        (
            "from worker.bus import lease, outbox",
            ["worker", "domain"],
            {
                "worker.bus.lease",
                "worker.bus.outbox",
            },
        ),
        ("from ..runtime import supervisor", ["worker", "domain"], {"worker.runtime.supervisor"}),
        ("from . import outcome", ["worker", "domain"], {"worker.domain.outcome"}),
        (
            "from .lyrics.text import parse",
            ["worker", "domain"],
            {"worker.domain.lyrics.text.parse"},
        ),
        ("from nats import *", ["worker", "bus"], {"nats"}),
    ],
)
def test_import_resolution_sees_every_form(
    source: str, package: list[str], expected: set[str]
) -> None:
    assert imports_in(source, package) == expected


def test_domain_rule_catches_relative_and_package_imports() -> None:
    for source in ("from ..runtime import supervisor", "from worker import bus"):
        names = imports_in(source, ["worker", "domain"])
        assert any(is_under(name, DOMAIN_FORBIDDEN) for name in names), source


@pytest.mark.parametrize("path", module_files(), ids=module_name)
def test_native_code_only_in_engine_process(path: Path) -> None:
    name = module_name(path)
    native = {imp for imp in imports_of(path) if is_under(imp, NATIVE_PACKAGES)}
    if not is_engine_module(name):
        assert not native, f"{name} imports native code {sorted(native)} outside the engine"


@pytest.mark.parametrize("path", module_files(), ids=module_name)
def test_main_process_never_imports_engine_modules(path: Path) -> None:
    name = module_name(path)
    if is_engine_module(name):
        return
    leaked = {imp for imp in imports_of(path) if is_engine_module(imp)}
    assert not leaked, f"{name} imports engine-process modules {sorted(leaked)}"


@pytest.mark.parametrize(
    "path", [p for p in module_files() if module_name(p).startswith("worker.bus")], ids=module_name
)
def test_bus_imports_only_contract_outcome_and_counters(path: Path) -> None:
    worker_imports = {imp for imp in imports_of(path) if imp.startswith("worker")}
    stray = {imp for imp in worker_imports if not is_under(imp, BUS_ALLOWED)}
    assert not stray, f"{module_name(path)} imports {sorted(stray)}"


@pytest.mark.parametrize(
    "path", [p for p in module_files() if module_name(p).startswith("worker.bus")], ids=module_name
)
def test_bus_never_creates_consumers(path: Path) -> None:
    forbidden = attribute_calls(path) & CONSUMER_CREATING_CALLS
    assert not forbidden, f"{module_name(path)} calls {sorted(forbidden)} (I5)"


@pytest.mark.parametrize(
    "path",
    [p for p in module_files() if module_name(p).startswith("worker.domain")],
    ids=module_name,
)
def test_domain_knows_no_nats_bus_or_runtime(path: Path) -> None:
    stray = {imp for imp in imports_of(path) if is_under(imp, DOMAIN_FORBIDDEN)}
    assert not stray, f"{module_name(path)} imports {sorted(stray)}"


@pytest.mark.parametrize(
    "path",
    [p for p in module_files() if module_name(p).startswith("worker.runtime")],
    ids=module_name,
)
def test_runtime_knows_no_domain_or_bus(path: Path) -> None:
    stray = {imp for imp in imports_of(path) if is_under(imp, RUNTIME_FORBIDDEN)}
    assert not stray, f"{module_name(path)} imports {sorted(stray)}"


@pytest.mark.parametrize(
    "path",
    [p for p in module_files() if module_name(p).startswith("worker.models")],
    ids=module_name,
)
def test_models_import_no_bus_domain_or_nats(path: Path) -> None:
    stray = {
        imp
        for imp in imports_of(path)
        if is_under(imp, ("nats", "worker.bus", "worker.domain", "worker.app"))
    }
    assert not stray, f"{module_name(path)} imports {sorted(stray)}"


TRAP_SCRIPT = """
import importlib, sys, types

class Trap(types.ModuleType):
    def __getattr__(self, attribute):
        raise AssertionError(self.__name__ + "." + attribute + " touched in the main process")

for name in NATIVE:
    sys.modules[name] = Trap(name)
for module in MODULES:
    importlib.import_module(module)
print("ok")
"""


def test_main_process_modules_import_without_torch() -> None:
    modules = sorted(
        module_name(path) for path in module_files() if not is_engine_module(module_name(path))
    )
    assert "worker.app" in modules
    script = f"NATIVE = {list(NATIVE_PACKAGES)!r}\nMODULES = {modules!r}\n{TRAP_SCRIPT}"
    result = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "ok"


LOADED_SCRIPT = """
import importlib, sys

for module in MODULES:
    importlib.import_module(module)
print(sorted(name for name in DEFERRED if name in sys.modules))
"""


def test_main_process_loads_tokenizer_models_only_on_first_use() -> None:
    modules = sorted(
        module_name(path) for path in module_files() if not is_engine_module(module_name(path))
    )
    script = f"DEFERRED = {list(LOADED_ON_FIRST_USE)!r}\nMODULES = {modules!r}\n{LOADED_SCRIPT}"
    result = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "[]"
