"""Summarize mixed-backend qualification; plots require NumPy and Matplotlib.

Usage: python scripts/qualification/analyze-mixed-i16.py /path/to/mixed-affine-20261028
Generated CSV/JSON/plots stay alongside the measured artifacts.
"""
import csv
from collections import Counter
import gzip
import json
import math
from pathlib import Path
import re
import statistics
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def completed_lives(snapshots):
    """Read the qualifier's existing bounded Debug records, checking completeness."""
    records = []
    fields = ("alive_seconds", "food", "kills", "life_net_points", "life_gross_earned", "life_boost_spent", "final_length", "peak_length")
    for snapshot in snapshots:
        chunks = re.findall(r"EvolvedLifeEndRecord \{([^{}]*)\}", snapshot["lifeEnds"])
        assert len(chunks) == snapshot["recentLifeEnds"] == snapshot["deaths"]
        for chunk in chunks:
            cause = re.search(r"\bcause: ([A-Za-z][A-Za-z0-9_]*)", chunk)
            assert cause is not None
            record = {"cause": cause.group(1)}
            for field in fields:
                match = re.search(r"\b" + field + r": (-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)", chunk)
                assert match is not None
                record[field] = float(match.group(1))
                assert math.isfinite(record[field])
            records.append(record)
    return records


def main():
    root = Path(sys.argv[1]).resolve()
    read = lambda name: json.loads((root / name).read_text())
    report = read("evolved-inference/report.json")
    assert report["candidateBackend"] == "rust-i16-affine-fma128-sse2-v1"
    assert report["brains"] == 85 and report["workers"] == 7 and report["deliveries"] == 10000
    assert report["requireExactRecurrent"] and report["cacheBytes"] == 59513600
    assert report["sourceCohort"]["sourceGeneration"] == 21
    assert report["graphLayoutSha256"] == "87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0"
    assert {row["cohortSlot"] for row in report["perGenome"]} == set(range(85))
    for index, width in enumerate([2, 792, 2, 792]):
        assert report["errors"][index]["samples"] == 85 * 10000 * width
        if index in (1, 3):
            assert report["errors"][index]["max"] == 0
            assert all(row["errors"][index]["max"] == 0 for row in report["perGenome"])
    matrices = read("evolved-inference/matrices.json")
    assert len(matrices) == 85 and all(len(row["matrices"]) == 4 for row in matrices)
    assert all(sum(matrix["length"] for matrix in row["matrices"]) == 350072 for row in matrices)
    timings = {}
    trials = {}
    for row in report["timings"]:
        timings.setdefault(row["backend"], []).append(row["msPerPopulationPass"])
        trials[(row["repetition"], row["backend"])] = row["msPerPopulationPass"]
    baseline = statistics.median(timings["rust-sse2-v1"])
    timing_summary = {}
    for backend, values in timings.items():
        assert len(values) == 8
        median = statistics.median(values)
        timing_summary[backend] = {"samplesMs": values, "medianMs": median, "minMs": min(values), "maxMs": max(values),
                                   "populationPassesPerSecond": 1000 / median, "speedupVsSse2Median": baseline / median,
                                   "pairedSpeedupSamples": [trials[(i, "rust-sse2-v1")] / trials[(i, backend)] for i in range(8)]}

    # Validate every slot/delivery and preserve an aggregate time series.
    maxima = np.zeros((85, 10000))
    mae = np.zeros((85, 10000))
    seen = np.zeros((85, 10000), dtype=bool)
    source = root / "evolved-inference/delivery-errors.csv"
    opener = source.open if source.exists() else lambda **kwargs: gzip.open(str(source) + ".gz", "rt", **kwargs)
    with opener(newline="") as stream:
        for row in csv.DictReader(stream):
            slot, delivery = int(row["cohort_slot"]), int(row["delivery"]) - 1
            assert 0 <= slot < 85 and 0 <= delivery < 10000 and not seen[slot, delivery]
            seen[slot, delivery] = True
            for column in ("paired_state_max", "feedback_state_max"):
                assert float(row[column]) == 0
            maxima[slot, delivery] = float(row["feedback_controller_max"])
            mae[slot, delivery] = float(row["feedback_controller_mae"])
            assert float(row["paired_controller_max"]) == maxima[slot, delivery]
    assert seen.all() and np.isfinite(maxima).all() and np.isfinite(mae).all()
    assert math.isclose(float(mae.mean()), report["errors"][2]["mae"], rel_tol=1e-12)
    assert float(maxima.max()) == report["errors"][2]["max"]
    prior = json.loads((root.parent / "feedback-investigation-20261028/full/report.json").read_text())
    assert prior["brains"] == 85 and prior["deliveries"] == 10000
    for genome in prior["results"]:
        previous = genome["totals"][2][0]  # Prior cold affine-only ablation vs SSE2.
        current = report["perGenome"][genome["slot"]]["errors"][2]
        for metric in ("samples", "mae", "rmse", "max"):
            assert math.isclose(previous[metric], current[metric], rel_tol=1e-12, abs_tol=1e-15), (genome["slot"], metric)
    with (root / "aggregate-delivery-errors.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["delivery", "controller_mae", "controller_max", "recurrent_max"])
        writer.writerows((i + 1, float(mae[:, i].mean()), float(maxima[:, i].max()), 0) for i in range(10000))

    engine = {}
    for label in ("sse2", "mixed", "i16"):
        name = f"real-{label}-w7"
        game = read(name + ".json")
        resources = read(name + "-receipt.json")
        assert game["admitted"] and game["completedSteps"] == 60300 and game["measuredSteps"] == 60000
        assert game["sourceSha256"] == report["sourceSha256"]
        assert game["graphLayoutSha256"] == report["graphLayoutSha256"]
        assert game["evolvedPopulation"]["logicalRootSha256"] == report["sourceCohort"]["logicalRootSha256"]
        assert game["sensorContract"]["inputSize"] == 264 and game["sensorContract"]["bins"] == 16
        boundaries = [row["before"] for row in game["gameplayBoundaries"] if "before" in row]
        boundaries.append(game["gameplayBoundaries"][-1])
        outcomes = [row for boundary in boundaries for row in boundary["populationOutcomes"]]
        lives = completed_lives(boundaries)
        engine[label] = {"realClock": game["realClock"], "stepCost": game["stepCost"],
                         "meanStepMs": game["stepCost"]["totalMs"] / game["measuredSteps"],
                         "nativePhaseCosts": game["nativePhaseCosts"], "physicsPhaseCosts": game["physicsPhaseCosts"],
                         "peakRssBytes": game["peakRssBytes"], "aggregateCpuStealFraction": resources["aggregateCpuStealFraction"],
                         "observations": sum(row["observations"] for row in game["consumerWork"]),
                         "incompleteObservations": sum(row["incompleteObservations"] for row in game["consumerWork"]),
                         "bodyCappedObservations": game["bodyCappedObservations"], "pelletCappedObservations": game["pelletCappedObservations"],
                         "boundaryLifecycleCounters": [{key: b[key] for key in ("generation", "lives", "deaths", "respawns")} for b in boundaries],
                         "completedPopulationLives": len(lives), "populationDeathCauses": dict(Counter(row["cause"] for row in lives)),
                         "completedLifeMeans": {key: statistics.fmean(row[key] for row in lives) if lives else None
                                                for key in ("alive_seconds", "food", "kills", "life_net_points", "life_gross_earned", "life_boost_spent", "final_length", "peak_length")},
                         "sourceSnapshotMeans": {key: statistics.fmean(row[key] for row in outcomes) for key in ("ageSeconds", "food", "points", "fitness", "kills", "length")},
                         "behaviorScope": "Source population snapshots, including 5-second warmup; closed-loop worlds evolve independently. No behavioral-equivalence claim."}
    one, seven = read("controlled-mixed-w1.json"), read("controlled-mixed-w7.json")
    assert one["continuationSha256"] == seven["continuationSha256"]
    summary = {"sourceCommit": read("configuration.json")["sourceCommit"], "sourceSha256": report["sourceSha256"],
               "rustc": report["rustc"], "target": report["target"], "buildContractSha256": report["buildContractSha256"],
               "candidateBackend": report["candidateBackend"], "defaultBackend": "rust-sse2-v1", "graphLayoutSha256": report["graphLayoutSha256"],
               "sourceCohort": report["sourceCohort"], "benchmark": timing_summary, "conversionMs": report["conversionMs"],
               "cacheBytes": report["cacheBytes"], "canonicalBytes": report["canonicalBytes"], "errorOrder": report["errorOrder"], "errors": report["errors"],
               "recurrentBitsEqualEveryDelivery": True, "previousAffineAblationPerGenomeMomentsMatch": True,
               "genomesControllerMaxAbove001": int(np.count_nonzero(maxima.max(axis=1) > .01)),
               "sensitiveSlots": {str(slot): report["perGenome"][slot] for slot in (24, 30, 35)}, "deliveryWindows": report["deliveryWindows"],
               "worker1And7ContinuationMatched": True, "engine": engine}
    (root / "analysis-summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    labels = {"rust-sse2-v1": "F32 SSE2", "benchmark-f32-fma128-v1": "F32 FMA128", "rust-i16-matrix-fma128-v1": "Full matrix-i16", "rust-i16-affine-fma128-sse2-v1": "Mixed affine-i16"}
    fig, axes = plt.subplots(1, 2, figsize=(12, 4.5), layout="constrained")
    for position, (backend, row) in enumerate(timing_summary.items()):
        label = labels.get(backend, backend)
        axes[0].scatter([position] * 8, row["samplesMs"], color="#286a9d", alpha=.65)
        axes[0].plot([position-.2, position+.2], [row["medianMs"]] * 2, color="#c74335", linewidth=3)
        axes[0].text(position, row["maxMs"] + .3, f'{row["speedupVsSse2Median"]:.3f}×', ha="center")
    axes[0].set_xticks(range(len(timing_summary)), [labels.get(k, k) for k in timing_summary], rotation=15)
    axes[0].set_ylabel("ms per 85-genome population pass (lower is better)")
    axes[0].set_title("7 workers · 8 balanced trials · 500 passes/trial")
    names = list(engine)
    axes[1].bar(names, [engine[k]["meanStepMs"] for k in names], color=["#777777", "#286a9d", "#b47a20"])
    axes[1].set_ylabel("Mean whole-engine step time (ms)")
    axes[1].set_title("1,000 measured simulation seconds per backend")
    fig.savefig(root / "performance.png", dpi=180)
    fig.savefig(root / "performance.svg")
    plt.close(fig)
    fig, axes = plt.subplots(2, 1, figsize=(12, 7), layout="constrained")
    x = np.arange(1, 10001)
    axes[0].plot(x, maxima.max(axis=0), color="#286a9d", linewidth=.7, label="All-genome component max")
    axes[0].plot(x, mae.mean(axis=0), color="#b47a20", linewidth=.7, label="All-genome MAE")
    for slot in (24, 30, 35):
        axes[1].plot(x, maxima[slot], linewidth=.65, label=f"Slot {slot}")
    for ax in axes:
        ax.set_ylabel("Controller absolute error vs SSE2")
        ax.set_xlabel("Delivery (identical repeated Motion/B16/ray observations)")
        ax.legend()
    axes[0].set_title("Mixed backend: all 85 hidden/cell trajectories bit-exact with SSE2 at every delivery")
    fig.savefig(root / "feedback-errors.png", dpi=180)
    fig.savefig(root / "feedback-errors.svg")
    plt.close(fig)
    print(json.dumps({"benchmark": timing_summary, "errors": report["errors"], "engine": engine}, indent=2))


if __name__ == "__main__":
    main()
