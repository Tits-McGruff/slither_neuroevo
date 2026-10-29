"""Reduce paired evolution artifacts without treating generations as replicates.

Writes compact machine-readable summaries and per-generation CSV. Optional
standard matplotlib figures are generated artifacts, not committed payloads.
"""
import argparse
import csv
import itertools
import json
import math
from pathlib import Path
import random
import statistics


def mean(values):
    values = list(values)
    return statistics.fmean(values) if values else None


def percentile(values, fraction):
    values = sorted(values)
    x = (len(values) - 1) * fraction
    lo, hi = math.floor(x), math.ceil(x)
    return values[lo] * (hi - x) + values[hi] * (x - lo) if lo != hi else values[lo]


def paired(values):
    if not values:
        return None
    n = len(values)
    effect = mean(values)
    rng = random.Random(20261029)
    boot = [mean(rng.choices(values, k=n)) for _ in range(20000)]
    permutation = [abs(mean(v * s for v, s in zip(values, signs))) for signs in itertools.product((-1, 1), repeat=n)]
    return {"seedPairs": n, "pairedDifferences": values, "meanDifference": effect,
            "medianDifference": statistics.median(values), "range": [min(values), max(values)],
            "exploratoryBootstrap95PercentInterval": [percentile(boot, .025), percentile(boot, .975)],
            "twoSidedExactSignFlipP": mean(v >= abs(effect) - 1e-12 for v in permutation),
            "warning": "Seed pair is the unit; small n, exploratory interval and symmetric-null sign-flip inference, no noninferiority claim."}


def reduce_generation(seed, backend, row):
    outcomes = row["outcomes"]
    matrices = [m for g in row["population"]["genomes"] for m in g["matrices"]]
    affine = [m for m in matrices if m["biasSharesScale"]]
    samples = row["population"]["sampledControllerComparisons"]
    q = row["population"]["aggregateQuantization"]
    consumers = row["nonterminalConsumerWork"]
    phases = row["nativePhaseCosts"]
    result = {"seed": seed, "backend": backend, "generation": row["generation"],
        "fitnessMean": row["fitness"]["mean"], "fitnessBest": row["fitness"]["max"], "fitnessMedian": row["fitness"]["median"],
        "scoreMean": mean(v["assessmentPoints"] for v in outcomes), "scoreBest": max(v["assessmentPoints"] for v in outcomes),
        "scoreMedian": statistics.median(v["assessmentPoints"] for v in outcomes), "foodMean": mean(v["assessmentFood"] for v in outcomes),
        "lifetimeMeanSeconds": mean(v["assessmentLifetimeSeconds"] for v in outcomes), "netPointsMean": mean(v["generationNetPoints"] for v in outcomes),
        "survivalCensoredFraction": mean(v["censored"] for v in outcomes), "completedLives": row["totalCompletedLives"],
        "livesStarted": sum(v["livesStarted"] for v in outcomes), "successfulRespawns": sum(v["successfulRespawns"] for v in outcomes),
        "simulatedSeconds": row["simulatedSeconds"], "normalEngineWallSeconds": row["normalEngineWallSeconds"],
        "nativeServiceWallSeconds": row["nativeServiceWallSeconds"], "handoffWallSeconds": row["checkpointAndHandoffWallSeconds"],
        "diagnosticWallSeconds": row["sampleCollectionWallSeconds"] + row["boundaryDiagnosticWallSeconds"] + row["checkpointRetentionWallSeconds"],
        "speciesCount": row["speciesCount"], "topSpeciesSize": row["topSpeciesSize"],
        "uniqueCanonicalGenomes": row["population"]["uniqueCanonicalWeightHashes"],
        "sampledCoordinateVariance": row["population"]["diversity"]["meanCoordinateVariance"],
        "effectiveSelectedParents": row["reproduction"]["effectiveParentCount"], "uniqueSelectedParents": row["reproduction"]["uniqueSelectedParents"],
        "eliteCopies": row["reproduction"]["eliteCopies"], "offspring": row["reproduction"]["offspring"], "selfParentPairs": row["reproduction"]["selfParentPairs"],
        "quantizationMae": q["mae"], "quantizationRmse": q["rmse"], "quantizationRelativeRmse": q["relativeRmse"], "quantizationMaxError": q["maxError"],
        "clippingCount": q["clippingCount"], "endpointCount": q["symmetricEndpointCount"], "forbiddenMagnitudeCount": sum(m["forbiddenMagnitudeCount"] for m in matrices),
        "matrixScaleMedian": statistics.median(m["scale"] for m in matrices), "matrixScaleMax": max(m["scale"] for m in matrices),
        "matrixCanonicalMaxAbs": max(m["maxAbs"] for m in matrices), "matrixMaxToP99Max": max(m["maxToP99"] or 0 for m in matrices),
        "affineBiasDominantFraction": mean(m["scaleSetter"] == "bias" for m in affine),
        "affineBiasTiedFraction": mean(m["scaleSetter"] == "tie" for m in affine), "affineBiasScaleInflationMax": max(m["biasScaleInflation"] or 0 for m in affine),
        "sampledComparisons": len(samples), "sampledControllerMaxError": max((v["controllerError"]["max"] for v in samples), default=None),
        "sampledControllerMeanMae": mean(v["controllerError"]["mae"] for v in samples),
        "sampledControllerRmse": math.sqrt(mean(v["controllerError"]["rmse"] ** 2 for v in samples)) if samples else None,
        "neuralDeliveriesExcludingTerminal": consumers[0]["observations"], "baselineDeliveriesExcludingTerminal": consumers[1]["observations"],
        "inferenceSeconds": phases["inferenceMicros"] / 1e6, "sensingSeconds": phases["sensingMicros"] / 1e6,
        "evolutionReproductionSeconds": row["evolutionReproductionMicros"] / 1e6, "evolutionSummarySeconds": row["evolutionSummaryMicros"] / 1e6}
    result["inferenceMicrosPerNonterminalDelivery"] = phases["inferenceMicros"] / consumers[0]["observations"] if consumers[0]["observations"] else None
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--plots", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(exist_ok=False)
    configuration = json.loads((args.artifacts / "configuration.json").read_text())
    runs, rows, matrix_rows = {}, [], []
    for directory in sorted(args.artifacts.glob("seed-*")):
        if not directory.is_dir() or not (directory / "evolution/generations.jsonl").exists():
            continue
        _, seed, backend = directory.name.split("-", 2)
        source = [json.loads(line) for line in (directory / "evolution/generations.jsonl").read_text().splitlines()]
        reduced = [reduce_generation(int(seed), backend, r) for r in source]
        rows.extend(reduced)
        if reduced:
            runs[(int(seed), backend)] = reduced
        for raw in source:
            grouped = {}
            for genome in raw["population"]["genomes"]:
                for m in genome["matrices"]:
                    grouped.setdefault((m["node"], m["matrixWithinNode"]), []).append(m)
            for (node, index), group in grouped.items():
                matrix_rows.append({"seed": int(seed), "backend": backend, "generation": raw["generation"], "node": node, "matrixWithinNode": index,
                    "scaleMedian": statistics.median(m["scale"] for m in group), "scaleMax": max(m["scale"] for m in group),
                    "canonicalMaxAbs": max(m["maxAbs"] for m in group), "relativeRmseMean": mean(m["relativeRmse"] for m in group),
                    "maxToP99Max": max(m["maxToP99"] or 0 for m in group), "biasSetters": sum(m["scaleSetter"] == "bias" for m in group),
                    "ties": sum(m["scaleSetter"] == "tie" for m in group), "clippingCount": sum(m["clippingCount"] for m in group),
                    "endpointCount": sum(m["symmetricEndpointCount"] for m in group)})
    for name, values in (("generations.csv", rows), ("matrices.csv", matrix_rows)):
        if values:
            with (args.output / name).open("w", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=list(values[0]))
                writer.writeheader()
                writer.writerows(values)
    seeds = sorted({seed for seed, _ in runs})
    pair_seeds = [seed for seed in seeds if all(len(runs.get((seed, b), [])) == 20 for b in ("sse2", "i16-matrix-fma128"))]
    metrics = ["fitnessMean", "fitnessBest", "fitnessMedian", "scoreMean", "foodMean", "lifetimeMeanSeconds", "netPointsMean", "survivalCensoredFraction", "completedLives", "speciesCount", "sampledCoordinateVariance", "effectiveSelectedParents"]
    comparisons = {}
    for window, start in (("all20", 0), ("last5", 15)):
        comparisons[window] = {metric: paired([mean(r[metric] for r in runs[(seed, "i16-matrix-fma128")][start:]) - mean(r[metric] for r in runs[(seed, "sse2")][start:]) for seed in pair_seeds]) for metric in metrics}
    performance = []
    for seed in pair_seeds:
        values = {}
        for backend in ("sse2", "i16-matrix-fma128"):
            run = runs[(seed, backend)]
            receipt = json.loads((args.artifacts / f"seed-{seed}-{backend}-receipt.json").read_text())
            native_report = json.loads((args.artifacts / f"seed-{seed}-{backend}/report.json").read_text())
            values[backend] = {"normalEngineWallSeconds": sum(r["normalEngineWallSeconds"] for r in run), "simulationSeconds": sum(r["simulatedSeconds"] for r in run),
                "diagnosticWallSeconds": sum(r["diagnosticWallSeconds"] for r in run), "fullProcessWallSeconds": receipt["wallSeconds"],
                "osPeakRssBytes": native_report["peakRssBytes"], "sampledPeakRssBytes": receipt["sampledPeakRssBytes"],
                "cpuStealFraction": receipt["aggregateCpuStealFraction"],
                "neuralDeliveriesExcludingTerminal": sum(r["neuralDeliveriesExcludingTerminal"] for r in run), "inferenceSeconds": sum(r["inferenceSeconds"] for r in run)}
        a, b = values["sse2"], values["i16-matrix-fma128"]
        performance.append({"seed": seed, "backends": values, "simulationThroughputSpeedup": (a["normalEngineWallSeconds"] / a["simulationSeconds"]) / (b["normalEngineWallSeconds"] / b["simulationSeconds"]),
            "inferenceCostPerDeliverySpeedup": (a["inferenceSeconds"] / a["neuralDeliveriesExcludingTerminal"]) / (b["inferenceSeconds"] / b["neuralDeliveriesExcludingTerminal"])})
    summary = {"configuration": configuration, "completedPairs": len(pair_seeds), "complete": len(pair_seeds) == 5, "generationsRecorded": len(rows),
        "comparisons": comparisons, "performance": performance,
        "pairedLateFitness": [{"seed": seed, **{b: mean(r["fitnessMean"] for r in runs[(seed, b)][15:]) for b in ("sse2", "i16-matrix-fma128")}} for seed in pair_seeds],
        "quantizationTrends": [{"seed": seed, "backend": backend, "first": {k: run[0][k] for k in ("quantizationRelativeRmse", "matrixScaleMax", "matrixMaxToP99Max", "affineBiasDominantFraction", "affineBiasScaleInflationMax")},
            "last": {k: run[-1][k] for k in ("quantizationRelativeRmse", "matrixScaleMax", "matrixMaxToP99Max", "affineBiasDominantFraction", "affineBiasScaleInflationMax")},
            "totalClipping": sum(r["clippingCount"] for r in run), "totalForbiddenMagnitude": sum(r["forbiddenMagnitudeCount"] for r in run),
            "minimumUniqueCanonicalGenomes": min(r["uniqueCanonicalGenomes"] for r in run), "minimumSpecies": min(r["speciesCount"] for r in run),
            "sampledControllerMaxError": max((r["sampledControllerMaxError"] or 0 for r in run), default=0)} for (seed, backend), run in sorted(runs.items())]}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if args.plots and rows:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
        panels = (("fitnessMean", "Population mean fitness"), ("foodMean", "Mean food"), ("survivalCensoredFraction", "Alive / censored fraction"),
                  ("sampledCoordinateVariance", "Sampled weight diversity"), ("quantizationRelativeRmse", "Relative quantization RMSE"), ("affineBiasDominantFraction", "Affine matrices with bias setting scale"))
        fig, axes = plt.subplots(3, 2, figsize=(13, 12))
        for axis, (metric, label) in zip(axes.flat, panels):
            for backend, color in (("sse2", "#2166ac"), ("i16-matrix-fma128", "#b2182b")):
                for seed in seeds:
                    run = runs.get((seed, backend), [])
                    axis.plot([r["generation"] for r in run], [r[metric] for r in run], color=color, alpha=.22, linewidth=1)
                points = sorted({r["generation"] for r in rows if r["backend"] == backend})
                axis.plot(points, [mean(r[metric] for r in rows if r["backend"] == backend and r["generation"] == g) for g in points], color=color, linewidth=2.3, label=backend)
            axis.set(xlabel="Additional generation", ylabel=label)
            axis.grid(alpha=.2)
            if metric == "affineBiasDominantFraction":
                axis.set_ylim(0, 1)
        axes.flat[0].legend()
        status = "Complete: five seed pairs" if summary["complete"] else f"PARTIAL: {len(pair_seeds)}/5 completed seed pairs; {len(rows)} generation records"
        fig.suptitle("Sustained evolution: exact-396 mature cohort, 85 genomes, seven workers\n"
                     + status + "\nFaint lines: independent seeds; thick lines: available seed means")
        fig.text(.5, .006, "Weight quantization diagnostics use full-i16 on both populations; SSE2 weight errors are counterfactual.",
                 ha="center", fontsize=9)
        fig.tight_layout(rect=(0, .025, 1, 1))
        fig.savefig(args.output / "evolution-trends.png", dpi=160)
        fig.savefig(args.output / "evolution-trends.svg")
        plt.close(fig)
    print(json.dumps({"complete": summary["complete"], "completedPairs": len(pair_seeds), "generationRows": len(rows)}))


if __name__ == "__main__":
    main()
