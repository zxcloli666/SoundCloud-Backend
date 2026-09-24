from __future__ import annotations

import math
from collections.abc import Sequence

Z_95 = 1.959963984540054
DEFAULT_RHO = 0.1


def wilson_upper(successes: int, trials: float, z: float = Z_95) -> float:
    if trials <= 0:
        return 1.0
    share = successes / trials
    denominator = 1.0 + z * z / trials
    centre = share + z * z / (2.0 * trials)
    spread = z * math.sqrt(share * (1.0 - share) / trials + z * z / (4.0 * trials * trials))
    return min(1.0, (centre + spread) / denominator)


def n_min(max_share: float, z: float = Z_95) -> int:
    return math.ceil(z * z * (1.0 - max_share) / max_share)


def design_effect(mean_cluster_size: float, rho: float = DEFAULT_RHO) -> float:
    return 1.0 + (mean_cluster_size - 1.0) * rho


def n_eff(total: int, mean_cluster_size: float, rho: float = DEFAULT_RHO) -> float:
    return total / design_effect(mean_cluster_size, rho)


def accuracy_at(deltas_s: Sequence[float], threshold_s: float) -> float:
    if not deltas_s:
        return 0.0
    return sum(1 for delta in deltas_s if abs(delta) <= threshold_s) / len(deltas_s)


def median(values: Sequence[float]) -> float:
    if not values:
        return math.nan
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[middle]
    return (ordered[middle - 1] + ordered[middle]) / 2.0


def icc(clusters: Sequence[Sequence[int]]) -> float:
    sizes = [len(cluster) for cluster in clusters if cluster]
    if len(sizes) < 2 or sum(sizes) == len(sizes):
        return DEFAULT_RHO
    grand = sum(sum(cluster) for cluster in clusters) / sum(sizes)
    k = len(sizes)
    n0 = (sum(sizes) - sum(size * size for size in sizes) / sum(sizes)) / (k - 1)
    between = sum(
        len(cluster) * (sum(cluster) / len(cluster) - grand) ** 2 for cluster in clusters if cluster
    ) / (k - 1)
    within = sum(
        sum((value - sum(cluster) / len(cluster)) ** 2 for value in cluster)
        for cluster in clusters
        if cluster
    ) / max(1, sum(sizes) - k)
    if between + (n0 - 1) * within <= 0:
        return DEFAULT_RHO
    return max(0.0, (between - within) / (between + (n0 - 1) * within))
