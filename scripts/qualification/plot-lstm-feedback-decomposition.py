"""Plot captured local arithmetic versus propagated recurrent error."""
import argparse
import csv
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def read(path):
    records={}
    with path.open() as source:
        for row in csv.DictReader(source):
            key=(int(row["slot"]),row["mode"],int(row["delivery"]))
            records.setdefault(key,{})[row["gate"]]=row
    return records


def broken_line(deliveries, values):
    x=[]; y=[]
    for index,(delivery,value) in enumerate(zip(deliveries,values)):
        if index and delivery != deliveries[index-1]+1: x.append(np.nan);y.append(np.nan)
        x.append(delivery);y.append(max(value,1e-12))
    return x,y


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root",type=Path)
    args=parser.parse_args(); root=args.root.resolve()
    records=read(root/"internal-lstm-error-over-time.csv")
    prefix_records=read(root/"seed-32-internal-error-over-time.csv")
    main=json.loads((root/"internal-lstm-analysis.json").read_text())
    prefix=json.loads((root/"seed-32-internal-analysis.json").read_text())
    plt.rcParams.update({"font.size":9,"axes.grid":True,"grid.alpha":.2,"savefig.dpi":180})
    figure,axes=plt.subplots(4,2,figsize=(14,14))
    rows=[(24,"i16-all",records,main),(30,"i16-all",records,main),(35,"i16-all",records,main),
          (35,"i16-prefix-32-then-sse2",prefix_records,prefix)]
    for index,(slot,mode,data,report) in enumerate(rows):
        peak=report["controllerPeaks"][f"{slot}:{mode}"]["delivery"]
        left,right=axes[index]
        for candidate,color in (("f32-fma128","#0072B2"),(mode,"#D55E00")):
            deliveries=sorted(key[2] for key in data if key[:2]==(slot,candidate))
            for field,style,label in (("preactivation_local_max","--","fresh arithmetic"),("preactivation_propagated_max","-","propagated state")):
                values=[max(float(row[field]) for row in data[(slot,candidate,d)].values()) for d in deliveries]
                x,y=broken_line(deliveries,values)
                left.plot(x,y,style,color=color,lw=1.1,label=f"{candidate}: {label}")
        left.axvline(peak,color="#777777",lw=.8,alpha=.5)
        left.set(yscale="log",ylim=(1e-12,2),xlim=(0,10000),ylabel=f"Slot {slot}\nMax preactivation error")
        if index==3: left.set_title("Finite prefix: fresh error is exactly zero after delivery 32")
        else: left.set_title(f"Slot {slot}: captured onset and excursion windows")
        for gate,color in zip(("input","forget","output","candidate"),("#0072B2","#009E73","#CC79A7","#E69F00")):
            deliveries=sorted(key[2] for key in data if key[:2]==(slot,mode) and abs(key[2]-peak)<=32)
            values=[float(data[(slot,mode,d)][gate]["gate_total_max"]) for d in deliveries]
            right.plot(deliveries,values,label=gate+" gate",color=color,lw=1.2)
        hidden=[float(next(iter(data[(slot,mode,d)].values()))["hidden_max"]) for d in deliveries]
        right.plot(deliveries,hidden,color="#222222",label="hidden state",lw=1.4)
        right.axvline(peak,color="#777777",lw=.8)
        right.set(title=f"{mode}: controller peak at delivery {peak}",ylabel="Max component difference vs SSE2")
    for axis in axes[-1]:axis.set_xlabel("Delivery")
    axes[0,0].legend(fontsize=8,loc="lower right")
    axes[0,1].legend(fontsize=8)
    figure.suptitle("LSTM attribution: small fresh errors, much larger carried-state errors\nLeft panels show captured windows only; gaps are unobserved, zero is plotted at 1e-12",fontsize=14)
    figure.tight_layout(rect=(0,0,1,.95))
    for extension in ("png","pdf"):figure.savefig(root/f"lstm-error-decomposition.{extension}")
    plt.close(figure)


if __name__=="__main__":main()
