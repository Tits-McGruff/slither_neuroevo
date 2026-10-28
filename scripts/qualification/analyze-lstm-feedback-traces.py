"""Decompose retained LSTM traces into local arithmetic and propagated errors."""
import argparse
import csv
import gzip
import json
from pathlib import Path

import numpy as np

GATES = ("input", "forget", "output", "candidate")


def cells(rows, field):
    return np.asarray([row[field] for row in rows], dtype=np.float64)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--trace-subdir", default="trace")
    parser.add_argument("--slots", default="24,30,35")
    parser.add_argument("--output-prefix", default="internal-lstm")
    args = parser.parse_args()
    root = args.root.resolve()
    metadata = json.loads((root / args.trace_subdir / "report.json").read_text())
    expected = {r["slot"]:r["traceDeliveries"] for r in metadata["results"]}
    peaks = {}
    controller_peaks = {}
    crossings = {}
    counts = {}
    retained = []
    with (root / f"{args.output_prefix}-error-over-time.csv").open("w",newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["slot","delivery","mode","gate","input_dot_local_max","recurrent_dot_local_max",
                         "preactivation_local_max","preactivation_propagated_max","preactivation_total_max",
                         "gate_total_max","preactivation_sign_changes","cell_max","hidden_max","dominant_cell",
                         "reference_cell","candidate_cell","reference_hidden","candidate_hidden",
                         "memory_carry_delta","forget_gate_delta","candidate_drive_delta","input_gate_delta",
                         "cell_decomposition_residual","dominant_hidden_cell","hidden_cell_reference_previous_c",
                         "hidden_cell_candidate_previous_c","hidden_cell_reference_next_c","hidden_cell_candidate_next_c",
                         "hidden_cell_reference_next_h","hidden_cell_candidate_next_h","hidden_cell_reference_preactivation",
                         "hidden_cell_candidate_preactivation","hidden_cell_reference_gate","hidden_cell_candidate_gate"])
        for slot in map(int,args.slots.split(",")):
            cycle_history = {}
            with gzip.open(root / args.trace_subdir / f"slot-{slot:02}-lstm.jsonl.gz","rt") as source:
                for line in source:
                    step = json.loads(line)
                    counts[slot] = counts.get(slot,0)+1
                    reference = step["modes"][0]["lstm"]
                    ref_pre = cells(reference,"preactivation")
                    ref_gate = cells(reference,"gate")
                    ref_c = cells(reference,"next_cell")
                    ref_h = cells(reference,"next_hidden")
                    ref_previous_c = cells(reference,"previous_cell")
                    for mode in step["modes"][1:]:
                        own = mode["lstm"]
                        same_state_sse = mode["sseOnSameStateLstm"]
                        assert np.array_equal(cells(same_state_sse,"input_dot"),cells(reference,"input_dot"))
                        assert np.array_equal(cells(own,"bias"),cells(reference,"bias"))
                        pre = cells(own,"preactivation")
                        local_pre = pre-cells(same_state_sse,"preactivation")
                        propagated_pre = cells(same_state_sse,"preactivation")-ref_pre
                        gates = cells(own,"gate")
                        c = cells(own,"next_cell")
                        h = cells(own,"next_hidden")
                        previous_c = cells(own,"previous_cell")
                        dominant = int(np.argmax(np.abs(c-ref_c)))
                        dominant_hidden = int(np.argmax(np.abs(h-ref_h)))
                        # Exact real-number decomposition of the F32 gate/state
                        # values. The residual is the ordinary F32 update rounding.
                        memory = ref_gate[:,1]*(previous_c-ref_previous_c)
                        forget = (gates[:,1]-ref_gate[:,1])*previous_c
                        candidate = ref_gate[:,0]*(gates[:,3]-ref_gate[:,3])
                        input_gate = (gates[:,0]-ref_gate[:,0])*gates[:,3]
                        residual = (c-ref_c)-(memory+forget+candidate+input_gate)
                        input_local = cells(own,"input_dot")-cells(same_state_sse,"input_dot")
                        recurrent_local = cells(own,"recurrent_dot")-cells(same_state_sse,"recurrent_dot")
                        for gate,gatename in enumerate(GATES):
                            writer.writerow([slot,step["delivery"],mode["mode"],gatename,
                                np.abs(input_local[:,gate]).max(),np.abs(recurrent_local[:,gate]).max(),
                                np.abs(local_pre[:,gate]).max(),np.abs(propagated_pre[:,gate]).max(),np.abs(pre[:,gate]-ref_pre[:,gate]).max(),
                                np.abs(gates[:,gate]-ref_gate[:,gate]).max(),np.sum(np.signbit(pre[:,gate])!=np.signbit(ref_pre[:,gate])),
                                np.abs(c-ref_c).max(),np.abs(h-ref_h).max(),dominant,
                                ref_c[dominant],c[dominant],ref_h[dominant],h[dominant],
                                memory[dominant],forget[dominant],candidate[dominant],input_gate[dominant],residual[dominant],
                                dominant_hidden,ref_previous_c[dominant_hidden],previous_c[dominant_hidden],
                                ref_c[dominant_hidden],c[dominant_hidden],ref_h[dominant_hidden],h[dominant_hidden],
                                ref_pre[dominant_hidden,gate],pre[dominant_hidden,gate],ref_gate[dominant_hidden,gate],gates[dominant_hidden,gate]])
                        key = f"{slot}:{mode['mode']}"
                        peak = float(np.abs(c-ref_c).max())
                        if key not in peaks or peak > peaks[key]["cellMax"]:
                            peaks[key] = {"slot":slot,"mode":mode["mode"],"delivery":step["delivery"],"cellMax":peak,
                                "hiddenMax":float(np.abs(h-ref_h).max()),"cell":dominant,
                                "reference":reference[dominant],"candidate":own[dominant],"sseOnCandidateState":same_state_sse[dominant],
                                "maxLocalPreactivation":float(np.abs(local_pre).max()),"maxPropagatedPreactivation":float(np.abs(propagated_pre).max()),
                                "controller":mode["controller"],"referenceController":step["modes"][0]["controller"]}
                        controller_error = float(np.abs(np.array(mode["controller"])-np.array(step["modes"][0]["controller"])).max())
                        event = {"slot":slot,"mode":mode["mode"],"delivery":step["delivery"],"controllerMax":controller_error,
                            "hiddenRms":float(np.sqrt(np.mean((h-ref_h)**2))),"hiddenMax":float(np.abs(h-ref_h).max()),
                            "controller":mode["controller"],"referenceController":step["modes"][0]["controller"],
                            "maxLocalPreactivation":float(np.abs(local_pre).max()),"maxPropagatedPreactivation":float(np.abs(propagated_pre).max()),
                            "topHiddenCells":[{"cell":int(j),"reference":reference[j],"candidate":own[j],"sseOnCandidateState":same_state_sse[j],
                                "cellDeltaTerms":{"memoryCarry":float(memory[j]),"forgetGate":float(forget[j]),
                                    "candidateDrive":float(candidate[j]),"inputGate":float(input_gate[j]),"roundingResidual":float(residual[j])}}
                                for j in np.argsort(np.abs(h-ref_h))[-3:][::-1]]}
                        if key not in controller_peaks or controller_error > controller_peaks[key]["controllerMax"]:
                            controller_peaks[key] = event
                        for threshold in (1e-3,1e-2):
                            crossing_key = f"{key}:hidden-rms-{threshold}"
                            if crossing_key not in crossings and event["hiddenRms"] > threshold:
                                crossings[crossing_key] = event
                        if step["delivery"] % 32 == 0:
                            previous_cycle = cycle_history.get((step["delivery"]-32,mode["mode"]))
                            cycle_movement = None if previous_cycle is None else {
                                "candidateHiddenRms":float(np.sqrt(np.mean((h-previous_cycle[0])**2))),
                                "referenceHiddenRms":float(np.sqrt(np.mean((ref_h-previous_cycle[1])**2)))}
                            retained.append({"slot":slot,"delivery":step["delivery"],"mode":mode["mode"],
                                "cellMax":peak,"hiddenRms":float(np.sqrt(np.mean((h-ref_h)**2))),
                                "preactivationLocalMax":float(np.abs(local_pre).max()),"preactivationPropagatedMax":float(np.abs(propagated_pre).max()),
                                "movementSincePreviousObservationCycle":cycle_movement})
                            cycle_history[(step["delivery"],mode["mode"])] = (h,ref_h)
            print("internal traces analyzed",slot,counts[slot],flush=True)
            assert counts[slot] == expected[slot], (slot,counts[slot],expected[slot])
    (root / f"{args.output_prefix}-analysis.json").write_text(json.dumps({"traceDeliveries":counts,"peaks":peaks,"controllerPeaks":controller_peaks,
        "firstCapturedHiddenRmsCrossings":crossings,"observationCycleBoundaries":retained,
        "decomposition":"preactivation candidate - SSE reference = (candidate - SSE evaluated on candidate prior state) + (SSE on candidate state - SSE reference)",
        "cellDecomposition":"delta c = f_ref*delta previous_c + delta f*previous_c_candidate + i_ref*delta g + delta i*g_candidate + F32 rounding residual",
        "scope":"All gates/cells for every backend/ablation within explicit captured windows; peaks are peaks within those windows"},indent=2)+"\n")


if __name__ == "__main__": main()
