"""Analyze bounded per-delivery feedback CSVs (requires NumPy).

Thresholds describe an investigation, not backend admission tolerances. Persistent
onset is the first 128-delivery window with at least 96 deliveries above the
stated threshold. Its reported delivery is the beginning of that window.
"""
import argparse
import csv
import gzip
import json
from pathlib import Path

import numpy as np

PAIRS = ["f32-fma128:sse2", "i16-all:sse2", "i16-affine:sse2", "i16-lstm:sse2",
         "i16-lstm-input:sse2", "i16-lstm-recurrent:sse2", "i16-all:f32-fma128"]
METRICS = ["controller_mae", "controller_rmse", "controller_max", "hidden_mae", "hidden_rmse", "hidden_max",
           "cell_mae", "cell_rmse", "cell_max", "turn", "boost", "reference_turn", "reference_boost"]


def onset(values, threshold, width=128, required=96):
    mask = values > threshold
    cumulative = np.concatenate(([0], np.cumsum(mask)))
    counts = cumulative[width:] - cumulative[:-width]
    hits = np.flatnonzero(counts >= required)
    first = np.flatnonzero(mask)
    return {"threshold": threshold, "firstCrossing": int(first[0]+1) if first.size else None,
            "persistentWindowStart": int(hits[0]+1) if hits.size else None,
            "deliveriesAbove": int(mask.sum()), "fractionAbove": float(mask.mean()),
            "windowDeliveries": width, "requiredAbove": required}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="feedback-investigation-20261028")
    args = parser.parse_args()
    root = args.root.resolve()
    report = json.loads((root / "full/report.json").read_text())
    assert report["brains"] == 85 and report["deliveries"] == 10000 and report["workers"] == 7
    aggregate = np.zeros((7, 10000, 9))
    all_genomes = []
    windows = []
    with (root / "per-genome-100-delivery-windows.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["slot", "pair", "first_delivery", "last_delivery", *METRICS[:9]])
        for slot in range(85):
            path = root / f"full/slot-{slot:02}-errors.csv.gz"
            with gzip.open(path, "rt") as source:
                values = np.loadtxt(source, delimiter=",", skiprows=1, usecols=[0, 1, *range(3, 16)])
            assert values.shape == (70000, 15)
            assert np.all(values[:, 0] == slot)
            assert np.array_equal(values[:, 1], np.repeat(np.arange(1, 10001), 7))
            data = values[:, 2:].reshape(10000, 7, 13).transpose(1, 0, 2)
            assert np.isfinite(data).all()
            genome = {"slot": slot, "pairs": {}}
            for pair_index, pair in enumerate(PAIRS):
                row = data[pair_index]
                for metric in range(9):
                    column = row[:, metric]
                    if metric % 3 == 2:
                        aggregate[pair_index, :, metric] = np.maximum(aggregate[pair_index, :, metric], column)
                    else:
                        aggregate[pair_index, :, metric] += column**2 if metric % 3 == 1 else column
                entry = {}
                for metric in ("controller", "hidden", "cell"):
                    offset = METRICS.index(metric+"_mae")
                    entry[metric] = {"mae": float(row[:, offset].mean()), "rmse": float(np.sqrt(np.mean(row[:, offset+1]**2))),
                                     "max": float(row[:, offset+2].max()), "peakDelivery": int(row[:, offset+2].argmax()+1),
                                     "first1000Mae": float(row[:1000, offset].mean()), "last1000Mae": float(row[-1000:, offset].mean()),
                                     "onsets": [onset(row[:, offset+1], t) for t in (1e-4, 1e-3, 1e-2, 1e-1)]}
                genome["pairs"][pair] = entry
                for start in range(0, 10000, 100):
                    block = row[start:start+100, :9]
                    stats = [float(block[:, m].max()) if m % 3 == 2 else
                             float(np.sqrt(np.mean(block[:, m]**2))) if m % 3 == 1 else float(block[:, m].mean()) for m in range(9)]
                    writer.writerow([slot, pair, start+1, start+100, *stats])
            all_genomes.append(genome)
            if slot in (24, 30, 35):
                intervals = [(1, 16)]
                # Choose windows from the primary three-way comparison; capture
                # every ablation inside those same windows for direct inspection.
                for pair_index in range(2):
                    row = data[pair_index]
                    for metric, threshold in ((4, 1e-3), (4, 1e-2), (1, 1e-2)):
                        crossing = onset(row[:, metric], threshold)
                        start = crossing["persistentWindowStart"]
                        if start is not None:
                            intervals.append((max(1,start-32), min(10000,start+160)))
                        if pair_index < 2 and crossing["firstCrossing"] is not None:
                            first = crossing["firstCrossing"]
                            intervals.append((max(1,first-32), min(10000,first+64)))
                    peak = int(row[:, 2].argmax()+1)
                    if row[:, 2].max() > 0.01:
                        intervals.append((max(1,peak-32), min(10000,peak+32)))
                # Merge to prevent duplicate captures of overlapping regions.
                merged = []
                for first, last in sorted(intervals):
                    if merged and first <= merged[-1][1]+1:
                        merged[-1][1] = max(last, merged[-1][1])
                    else:
                        merged.append([first,last])
                windows.extend({"slot":slot,"first":first,"last":last} for first,last in merged)
            print("analyzed", slot, flush=True)
    for metric in range(9):
        if metric % 3 != 2:
            aggregate[:, :, metric] /= 85
            if metric % 3 == 1:
                aggregate[:, :, metric] = np.sqrt(aggregate[:, :, metric])
    with (root / "aggregate-error-over-time.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["pair", "delivery", *METRICS[:9]])
        for p, pair in enumerate(PAIRS):
            for step in range(10000):
                writer.writerow([pair,step+1,*aggregate[p,step]])
    summary = {"sourceCommit": json.loads((root / "full-receipt.json").read_text())["sourceCommit"],
               "nativeMetadata":report,"onsetDefinition":__doc__,"perGenome":all_genomes,"aggregate":{}}
    for p, pair in enumerate(PAIRS):
        summary["aggregate"][pair] = {metric: {"mae":float(aggregate[p,:,offset].mean()),
            "rmse":float(np.sqrt(np.mean(aggregate[p,:,offset+1]**2))), "max":float(aggregate[p,:,offset+2].max()),
            "genomesControllerMaxAbove001":sum(g["pairs"][pair]["controller"]["max"]>0.01 for g in all_genomes),
            "genomesControllerMaxAbove01":sum(g["pairs"][pair]["controller"]["max"]>0.1 for g in all_genomes)}
            for metric,offset in (("controller",0),("hidden",3),("cell",6))}
    old = json.loads((root.parent / "evolved-inference/report.json").read_text())
    actual = summary["aggregate"]["i16-all:sse2"]
    old_error = old["errors"]
    summary["originalReproduction"] = {"controllerMae": actual["controller"]["mae"], "originalControllerMae": old_error[2]["mae"],
        "controllerMax":actual["controller"]["max"],"originalControllerMax":old_error[2]["max"],
        "recurrentMae":(actual["hidden"]["mae"]+actual["cell"]["mae"])/2,"originalRecurrentMae":old_error[3]["mae"],
        "recurrentMax":max(actual["hidden"]["max"],actual["cell"]["max"]),"originalRecurrentMax":old_error[3]["max"]}
    (root / "analysis-summary.json").write_text(json.dumps(summary, indent=2)+"\n")
    (root / "trace-windows.json").write_text(json.dumps(windows, indent=2)+"\n")
    print(json.dumps({"aggregate":summary["aggregate"],"reproduction":summary["originalReproduction"],"traceDeliveries":sum(w["last"]-w["first"]+1 for w in windows)},indent=2))


if __name__ == "__main__":
    main()
