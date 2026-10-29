"""Analyze finite i16 prefixes followed only by canonical SSE2 (NumPy/Matplotlib)."""
import argparse
import gzip
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def load(path):
    with gzip.open(path,"rt") as source:
        return np.loadtxt(source,delimiter=",",skiprows=1,usecols=range(3,16)).reshape(10000,7,13)


def describe(data, offset):
    return {"mae":float(data[:,offset].mean()),"rmse":float(np.sqrt(np.mean(data[:,offset+1]**2))),
            "max":float(data[:,offset+2].max()),"peakIndex":int(data[:,offset+2].argmax())}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root",type=Path)
    args=parser.parse_args()
    root=args.root.resolve()
    results=[]
    plt.rcParams.update({"font.size":10,"axes.grid":True,"grid.alpha":.2,"savefig.dpi":180})
    figure,axes=plt.subplots(3,2,figsize=(14,11),sharex=True)
    for row,slot in enumerate((24,30,35)):
        full=load(root/f"full/slot-{slot:02}-errors.csv.gz")
        curves=[("continuous i16",full[:,1],"#D55E00"),("continuous F32-FMA128",full[:,0],"#0072B2")]
        for prefix,color in ((1,"#009E73"),(32,"#CC79A7")):
            data=load(root/f"seed-{prefix}/slot-{slot:02}-errors.csv.gz")
            # The two binary revisions differ only in cold prefix/label support.
            # Validate every component of the unchanged trajectories, and every
            # delivery of the prefix before the intentional arithmetic switch.
            assert np.array_equal(data[:,[0,2,3,4,5]],full[:,[0,2,3,4,5]])
            assert np.array_equal(data[:prefix,1],full[:prefix,1])
            candidate=data[:,1]
            entry={"slot":slot,"prefixDeliveries":prefix,"sseOnlyFromDelivery":prefix+1,
                   "seedHiddenRms":float(candidate[prefix-1,4]),"seedCellRms":float(candidate[prefix-1,7]),
                   "controller":describe(candidate,0),"hidden":describe(candidate,3),"cell":describe(candidate,6),
                   "afterPrefix":{name:describe(candidate[prefix:],offset) for name,offset in (("controller",0),("hidden",3),("cell",6))},
                   "first1000ControllerMae":float(candidate[:1000,0].mean()),"last1000ControllerMae":float(candidate[-1000:,0].mean()),
                   "first1000HiddenRms":float(np.sqrt(np.mean(candidate[:1000,4]**2))),"last1000HiddenRms":float(np.sqrt(np.mean(candidate[-1000:,4]**2)))}
            for metrics in (entry,entry["afterPrefix"]):
                for name in ("controller","hidden","cell"):
                    metrics[name]["peakDelivery"]=metrics[name].pop("peakIndex")+1+(prefix if metrics is entry["afterPrefix"] else 0)
            results.append(entry)
            curves.append((f"{prefix} i16 deliveries, then SSE2",candidate,color))
        for label,data,color in curves:
            for column,metric in enumerate((1,4)):
                block=data[:9984,metric].reshape(-1,32)
                axes[row,column].plot(np.arange(32,9985,32),np.maximum(np.sqrt(np.mean(block*block,axis=1)),1e-10),label=label,color=color,lw=1.2)
                axes[row,column].set_yscale("log")
                axes[row,column].set_ylim(1e-9,2)
        axes[row,0].set_ylabel(f"Slot {slot}\nController RMS error")
        axes[row,1].set_ylabel(f"Slot {slot}\nHidden-state RMS error")
    for axis in axes[-1]: axis.set_xlabel("Delivery (32 actual observations repeated)")
    axes[0,0].set_title("Finite state perturbations versus continuous changed arithmetic")
    axes[0,1].set_title("All curves compared with identical-initial-state SSE2")
    handles,labels=axes[0,0].get_legend_handles_labels()
    figure.legend(handles,labels,loc="lower center",ncol=2,bbox_to_anchor=(.5,.005))
    figure.tight_layout(rect=(0,.07,1,1))
    for extension in ("png","pdf"): figure.savefig(root/f"finite-prefix-feedback.{extension}")
    plt.close(figure)
    (root/"prefix-control-analysis.json").write_text(json.dumps({"results":results,
        "validation":"Unchanged comparator/ablation trajectories and all prefix deliveries match the original full comparison exactly",
        "interpretation":"After the prefix, both candidate and reference use the same canonical F32 weights and existing SSE2; only their recurrent states differ"},indent=2)+"\n")
    print(json.dumps(results,indent=2))


if __name__=="__main__": main()
