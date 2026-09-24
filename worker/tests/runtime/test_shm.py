from __future__ import annotations

import errno
import os

import numpy as np
import pytest

from worker.runtime import shm
from worker.runtime.protocol import ArrayRef


@pytest.mark.parametrize(
    "array",
    [
        np.arange(12, dtype=np.float32).reshape(3, 4),
        np.array([[1, 2], [3, 4]], dtype=np.int16),
        np.zeros((2, 0, 3), dtype=np.float64),
        np.array([True, False]),
        np.arange(6, dtype=np.float32).reshape(2, 3)[:, ::2],
    ],
    ids=["f32", "i16", "empty", "bool", "strided"],
)
def test_share_and_read_roundtrip(array: np.ndarray) -> None:
    blocks = shm.SharedBlocks(os.getpid(), 1, 7, "in")
    refs = blocks.share({"x": array})
    try:
        loaded = shm.read(refs["x"])
        assert loaded.shape == array.shape
        assert loaded.dtype == array.dtype
        np.testing.assert_array_equal(loaded, array)
        assert loaded.flags.c_contiguous
    finally:
        blocks.release()
    if refs["x"].shm_name:
        assert not (shm.SHM_DIR / refs["x"].shm_name).exists()


def test_release_unlinks_every_block() -> None:
    blocks = shm.SharedBlocks(os.getpid(), 2, 9, "in")
    refs = blocks.share({"a": np.ones(4), "b/c d": np.ones(2)})
    names = [ref.shm_name for ref in refs.values()]
    assert all((shm.SHM_DIR / name).exists() for name in names)
    assert names[1] == f"wk-{os.getpid()}-2-9-in-b_c_d"
    blocks.release()
    assert not any((shm.SHM_DIR / name).exists() for name in names)
    blocks.release()


def test_take_all_reads_then_unlinks() -> None:
    refs = shm.SharedBlocks(os.getpid(), 3, 1, "out").share({"v": np.arange(5)})
    arrays = shm.take_all(refs)
    np.testing.assert_array_equal(arrays["v"], np.arange(5))
    assert not (shm.SHM_DIR / refs["v"].shm_name).exists()


def test_read_missing_block_raises() -> None:
    with pytest.raises(FileNotFoundError):
        shm.read(ArrayRef(f"wk-{os.getpid()}-0-0-in-missing", (1,), "<f4"))


def test_object_arrays_are_rejected() -> None:
    with pytest.raises(ValueError, match="object arrays"):
        shm.SharedBlocks(os.getpid(), 4, 1, "in").share({"o": np.array([object()])})


def test_sweep_by_engine_prefix() -> None:
    main = os.getpid()
    mine = shm.SharedBlocks(main, 41, 1, "in").share({"x": np.ones(1)})
    other = shm.SharedBlocks(main, 42, 1, "in").share({"x": np.ones(1)})
    try:
        removed = shm.sweep(shm.engine_prefix(main, 41))
        assert removed == 1
        assert not (shm.SHM_DIR / mine["x"].shm_name).exists()
        assert (shm.SHM_DIR / other["x"].shm_name).exists()
    finally:
        shm.sweep(shm.engine_prefix(main, 42))


def test_sweep_dead_owners_keeps_live_owner() -> None:
    live = shm.SharedBlocks(os.getpid(), 5, 1, "in").share({"x": np.ones(1)})
    dead_name = f"{shm.PREFIX}999999999-5-1-in-x"
    (shm.SHM_DIR / dead_name).write_bytes(b"\0" * 8)
    try:
        assert shm.sweep_dead_owners() >= 1
        assert not (shm.SHM_DIR / dead_name).exists()
        assert (shm.SHM_DIR / live["x"].shm_name).exists()
    finally:
        shm.unlink(live["x"].shm_name)
        shm.unlink(dead_name)


def no_space(*_: object) -> None:
    raise OSError(errno.ENOSPC, "No space left on device")


def test_failed_write_leaves_no_file(monkeypatch: pytest.MonkeyPatch) -> None:
    name = f"{shm.PREFIX}{os.getpid()}-6-1-in-x"
    monkeypatch.setattr(shm.os, "ftruncate", no_space)
    with pytest.raises(OSError):
        shm.write(name, np.ones(4))
    assert not (shm.SHM_DIR / name).exists()


def test_full_shm_is_reported_as_oserror_before_any_page_is_touched(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    name = f"{shm.PREFIX}{os.getpid()}-6-2-in-x"
    monkeypatch.setattr(shm.os, "posix_fallocate", no_space)
    with pytest.raises(OSError):
        shm.write(name, np.ones(4))
    assert not (shm.SHM_DIR / name).exists()


def test_partial_share_releases_what_it_wrote(monkeypatch: pytest.MonkeyPatch) -> None:
    real_write = shm.write

    def second_fails(name: str, array: np.ndarray) -> ArrayRef:
        if name.endswith("-b"):
            no_space()
        return real_write(name, array)

    monkeypatch.setattr(shm, "write", second_fails)
    with pytest.raises(OSError):
        shm.SharedBlocks(os.getpid(), 6, 3, "in").share({"a": np.ones(4), "b": np.ones(4)})
    stem = f"{shm.PREFIX}{os.getpid()}-6-3-in-"
    assert not any(entry.name.startswith(stem) for entry in shm.SHM_DIR.iterdir())
