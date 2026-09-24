from __future__ import annotations

import argparse
import ctypes
import importlib
import os
import signal
import sys
import time
from dataclasses import dataclass, replace
from multiprocessing.connection import Connection

from worker.observability.logging import JsonLog, error_text
from worker.runtime import allocator, devices, shm
from worker.runtime.protocol import (
    BadInput,
    Call,
    Command,
    CommandKind,
    ErrorKind,
    ModelSlot,
    Pong,
    Reply,
    SlotSpec,
    SlotState,
)

EXIT_LOAD_FAILED = 3
EXIT_ORPHANED = 4
PR_SET_PDEATHSIG = 1


@dataclass(frozen=True)
class Options:
    fd: int
    owner: int
    onednn: bool
    threads: int
    release_after_call: bool
    oom_score_adj: int


class LoadFailed(Exception):
    pass


def main(argv: list[str]) -> int:
    options = parse(argv)
    log = JsonLog(component="engine")
    parent = os.getppid()
    if parent != options.owner:
        log.error("engine_orphaned_at_start", owner=options.owner, parent=parent)
        return EXIT_ORPHANED
    set_oom_score_adj(options.oom_score_adj, log)
    set_parent_death_signal(log)
    conn = Connection(options.fd)
    specs = conn.recv()
    engine = Engine(tuple(specs), options, log)
    return engine.serve(conn)


def parse(argv: list[str]) -> Options:
    parser = argparse.ArgumentParser(prog="worker.runtime.engine_main")
    parser.add_argument("--fd", type=int, required=True)
    parser.add_argument("--owner", type=int, required=True)
    parser.add_argument("--onednn", choices=("on", "off"), default="on")
    parser.add_argument("--threads", type=int, default=0)
    parser.add_argument("--release-after-call", choices=("on", "off"), default="on")
    parser.add_argument("--oom-score-adj", type=int, default=900)
    parsed = parser.parse_args(argv)
    return Options(
        fd=parsed.fd,
        owner=parsed.owner,
        onednn=parsed.onednn == "on",
        threads=parsed.threads,
        release_after_call=parsed.release_after_call == "on",
        oom_score_adj=parsed.oom_score_adj,
    )


def set_oom_score_adj(value: int, log: JsonLog) -> None:
    try:
        with open("/proc/self/oom_score_adj", "w", encoding="ascii") as handle:
            handle.write(str(value))
    except OSError as error:
        log.warning("engine_oom_score_adj_failed", value=value, error=str(error))


def set_parent_death_signal(log: JsonLog) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(PR_SET_PDEATHSIG, int(signal.SIGKILL), 0, 0, 0) != 0:
        log.warning("engine_pdeathsig_failed", errno=ctypes.get_errno())


def load_class(loader: str) -> type[ModelSlot]:
    module_name, _, class_name = loader.partition(":")
    if not module_name or not class_name:
        raise LoadFailed(f"loader must be 'module:Class', got {loader!r}")
    module = importlib.import_module(module_name)
    loaded: type[ModelSlot] = getattr(module, class_name)
    return loaded


class Slot:
    def __init__(self, spec: SlotSpec) -> None:
        self.spec = spec
        self.model: ModelSlot | None = None
        self.calls = 0

    @property
    def loaded(self) -> bool:
        return self.model is not None

    def state(self, reserved_mib: int, allocated_mib: int) -> SlotState:
        return SlotState(self.spec.name, self.loaded, self.calls, reserved_mib, allocated_mib)


class Engine:
    def __init__(self, specs: tuple[SlotSpec, ...], options: Options, log: JsonLog) -> None:
        self._slots = {spec.name: Slot(spec) for spec in specs}
        self._options = options
        self._log = log
        self._torch_configured = False

    def serve(self, conn: Connection) -> int:
        while True:
            try:
                message = conn.recv()
            except EOFError:
                self._log.info("engine_parent_gone")
                self._unload_all()
                return 0
            try:
                if isinstance(message, Command):
                    if message.kind is CommandKind.STOP:
                        self._unload_all()
                        conn.send(self._pong(message.id))
                        return 0
                    conn.send(self._handle(message))
                elif isinstance(message, Call):
                    conn.send(self._execute(message))
                else:
                    self._log.error("engine_unknown_message", kind=type(message).__name__)
            except LoadFailed as failed:
                self._log.error("engine_exit_load_failed", error=str(failed))
                return EXIT_LOAD_FAILED

    def _handle(self, command: Command) -> Pong:
        if command.kind is CommandKind.LOAD and command.slot is not None:
            self._load(command.slot)
        elif command.kind is CommandKind.UNLOAD and command.slot is not None:
            self._unload(command.slot)
        return self._pong(command.id)

    def _pong(self, message_id: int) -> Pong:
        reserved, allocated = allocator.memory_mib() if self._torch_configured else (0, 0)
        return Pong(
            message_id, tuple(slot.state(reserved, allocated) for slot in self._slots.values())
        )

    def _load(self, name: str) -> Slot:
        slot = self._slots.get(name)
        if slot is None:
            raise LoadFailed(f"unknown slot {name!r}")
        if slot.model is not None:
            return slot
        self._configure_torch(slot.spec)
        spec = self._placed(slot.spec)
        started = time.perf_counter()
        try:
            model = load_class(spec.loader)()
            model.load(spec)
            model.warmup()
        except Exception as error:
            self._log.exception("slot_load_failed", error, slot=name, loader=spec.loader)
            raise LoadFailed(f"{name}: {error_text(error)}") from error
        slot.model = model
        self._release()
        self._log.info(
            "slot_loaded",
            slot=name,
            model=spec.model,
            revision=spec.revision[:8],
            device=spec.device,
            seconds=round(time.perf_counter() - started, 2),
        )
        return slot

    def _unload(self, name: str) -> None:
        slot = self._slots.get(name)
        if slot is None or slot.model is None:
            return
        try:
            slot.model.unload()
        except Exception as error:
            self._log.exception("slot_unload_failed", error, slot=name)
        slot.model = None
        self._release()
        self._log.info("slot_unloaded", slot=name)

    def _unload_all(self) -> None:
        for name in self._slots:
            self._unload(name)

    def _configure_torch(self, spec: SlotSpec) -> None:
        if self._torch_configured or not devices.uses_torch(spec.device):
            return
        devices.configure(self._options.onednn, self._options.threads)
        self._torch_configured = True

    def _placed(self, spec: SlotSpec) -> SlotSpec:
        if not devices.uses_torch(spec.device):
            return spec
        return replace(spec, device=devices.resolve(spec.device))

    def _release(self) -> None:
        if self._torch_configured:
            allocator.release()

    def _execute(self, call: Call) -> Reply:
        started = time.perf_counter()
        slot = self._slots.get(call.slot)
        if slot is None:
            return Reply(
                call.id, error_kind=ErrorKind.MODEL_ERROR, error=f"unknown slot {call.slot}"
            )
        model = self._load(call.slot).model
        assert model is not None
        oom = False
        try:
            arrays = shm.read_all(call.arrays)
            out_arrays, result = model.invoke(call.method, arrays, dict(call.args))
            blocks = shm.SharedBlocks(self._options.owner, os.getpid(), call.id, "out")
            refs = blocks.share(out_arrays)
            reply = Reply(call.id, arrays=refs, result=dict(result))
        except BadInput as error:
            reply = Reply(call.id, error_kind=ErrorKind.BAD_INPUT, error=error_text(error))
        except Exception as error:
            oom = allocator.is_out_of_memory(error)
            kind = ErrorKind.OOM if oom else ErrorKind.MODEL_ERROR
            self._log.exception(
                "slot_call_failed", error, slot=call.slot, method=call.method, kind=str(kind)
            )
            reply = Reply(call.id, error_kind=kind, error=error_text(error))
        finally:
            slot.calls += 1
            if oom or self._options.release_after_call:
                self._release()
        duration_ms = (time.perf_counter() - started) * 1000.0
        return Reply(
            reply.id,
            arrays=reply.arrays,
            result=reply.result,
            error_kind=reply.error_kind,
            error=reply.error,
            duration_ms=duration_ms,
        )


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
