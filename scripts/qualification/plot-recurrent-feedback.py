"""Render the retained per-genome feedback study (NumPy and Matplotlib)."""
import argparse
import csv
import gzip
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    summary = json.loads((root / "analysis-summary.json").read_text())
    colors = ["#0072B2", "#D55E00", "#009E73", "#CC79A7", "#E69F00", "#56B4E9", "#555555"]
    pairs = list(summary["aggregate"])
    plt.rcParams.update({"font.size":10,"axes.grid":True,"grid.alpha":0.2,"savefig.dpi":180})
    figure, axes = plt.subplots(3,2,figsize=(14,12),sharex=True)
    for slot, axisrow in zip((24,30,35),axes):
        with gzip.open(root / f"full/slot-{slot:02}-errors.csv.gz", "rt") as source:
            values = np.loadtxt(source,delimiter=",",skiprows=1,usecols=range(3,12)).reshape(10000,7,9)
        for p,pair in enumerate(pairs[:6]):
            for axis,metric in zip(axisrow,(1,4)):
                # Exact RMS over nonoverlapping 32-delivery observation cycles.
                data = values[:9984,p,metric].reshape(-1,32)
                axis.plot(np.arange(32,9985,32),np.maximum(np.sqrt(np.mean(data*data,axis=1)),1e-10),label=pair.split(":")[0],color=colors[p],lw=1.2)
                axis.set_yscale("log")
                axis.set_ylim(1e-9,2)
        axisrow[0].set_ylabel(f"Slot {slot}\nController RMS error")
        axisrow[1].set_ylabel(f"Slot {slot}\nHidden-state RMS error")
    axes[0,0].set_title("Independent feedback against SSE2: controller")
    axes[0,1].set_title("Independent feedback against SSE2: hidden state")
    for axis in axes[-1]: axis.set_xlabel("Delivery (32-row actual observation bank repeated)")
    handles,labels=axes[0,0].get_legend_handles_labels()
    figure.legend(handles,labels,loc="lower center",ncol=3,bbox_to_anchor=(0.5,0.005))
    figure.suptitle("Exact-396 mature cohort: sensitive genomes\n85 genomes, 7 workers; no world feedback or default change",fontsize=15)
    figure.tight_layout(rect=(0,0.06,1,0.94))
    for extension in ("png","pdf"): figure.savefig(root / f"sensitive-genome-feedback.{extension}")
    plt.close(figure)
    figure,axes=plt.subplots(1,2,figsize=(13,5))
    genomes=summary["perGenome"]
    x=np.array([g["pairs"][pairs[0]]["controller"]["max"] for g in genomes])
    y=np.array([g["pairs"][pairs[1]]["controller"]["max"] for g in genomes])
    axes[0].scatter(np.maximum(x,1e-10),np.maximum(y,1e-10),s=22,alpha=.7)
    axes[0].set(xscale="log",yscale="log",xlabel="F32-FMA128 vs SSE2: controller max",ylabel="All-i16 vs SSE2: controller max",title="Every genome, 10,000 deliveries")
    for slot in (24,30,35): axes[0].annotate(str(slot),(x[slot],y[slot]),xytext=(5,4),textcoords="offset points")
    for p,pair in enumerate(pairs[:6]):
        values=np.array([g["pairs"][pair]["controller"]["max"] for g in genomes])
        axes[1].plot(np.arange(85),np.maximum(values,1e-10),".",ms=4,label=pair.split(":")[0],color=colors[p])
    axes[1].set(yscale="log",xlabel="Cohort slot",ylabel="Controller maximum error vs SSE2",title="Matrix-group ablations, all genomes")
    axes[1].legend(fontsize=8,ncol=2)
    figure.tight_layout()
    for extension in ("png","pdf"): figure.savefig(root / f"all-genome-ablation.{extension}")
    plt.close(figure)


if __name__ == "__main__": main()
