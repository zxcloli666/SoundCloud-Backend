from __future__ import annotations

from eval.manifest import Manifest, Track

from eval import manifest as manifest_module


def track(identifier: str, cluster: str, split: str = "calib", expected: str = "ok") -> Track:
    return Track(
        id=identifier,
        kind="positive" if expected == "ok" else "synthetic",
        split=split,
        language="ru",
        audio=f"audio/{cluster}.m4a",
        input_text=f"texts/{identifier}.txt",
        input_source="provider_plain",
        reference_lrc=None,
        expected=expected,
        cluster=cluster,
    )


def negatives(clusters: int, per_cluster: int) -> list[Track]:
    return [
        track(f"a{c}:n{n}", f"a{c}", "calib" if c % 2 else "control", "lyrics_mismatch")
        for c in range(clusters)
        for n in range(per_cluster)
    ]


def test_too_few_negatives_fail_the_manifest_check() -> None:
    small = Manifest(1, "a0", (track("a0", "a0"), track("a1", "a1", "control"), *negatives(2, 10)))
    assert any("n_eff" in problem for problem in manifest_module.check(small))


def test_sixty_clusters_of_ten_negatives_pass() -> None:
    tracks = (track("a0", "a0"), track("a1", "a1", "control"), *negatives(60, 10))
    assert manifest_module.negative_n_eff(list(tracks[2:])) > 300
    assert manifest_module.check(Manifest(1, "a0", tracks)) == []


def test_duplicate_ids_and_missing_audit_are_reported() -> None:
    tracks = (track("a0", "a0"), track("a0", "a0", "control"))
    problems = manifest_module.check(Manifest(1, "zz", tracks))
    assert "duplicate track ids" in problems
    assert "audit track is not in the manifest" in problems


def test_lrc_parsing_keeps_timestamps_and_skips_blank_lines() -> None:
    parsed = manifest_module.parse_lrc("[00:10.50]first\n[00:12.00]\n[01:01.250]second\nnoise")
    assert parsed == [(10.5, "first"), (61.25, "second")]
    assert manifest_module.plain_lines("a\n\n b \n") == ["a", "b"]
