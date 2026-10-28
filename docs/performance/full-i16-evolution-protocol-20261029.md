# Sustained SSE2 / full matrix-i16 evolutionary qualification

Protocol fixed before measured runs. Starting branch commit:
`a98e1aa3671e8b819acef588ccc4e23357f187dc`.
New experiment branch: `experiment/matrix-i16-evolution-20261029`.
SSE2 remains the default; production configuration and inference arithmetic are unchanged.

Use the preserved generation-21 population: 85 genomes, exact Input264 /
Dense396 + LSTM396 / Concat792 / MLP264,134,2, 1,397,096 F32 parameters each.
Checkpoint SHA256 `f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935`;
graph layout SHA256 `87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0`.
Motion/B16 with body/wall rays; seven calculation workers on the Ryzen 2700 VM.

Five independent paired seeds: 104729, 130363, 155921, 196613, 262147.
Each starts from all 85 canonical mature genomes, transferred into fresh lives
using the existing strict cohort loader. Native generation numbers restart at 1;
20 additional generations are measured for each backend and seed. Retain the
recorded 480-second generation limit and other recognized source settings.
The ordinary early-completion rule remains enabled if recorded in those settings.
Run only SSE2 and full matrix-i16, sequentially, alternating within-pair order.
Use the existing controlled-clock qualifier: ordinary complete 60 Hz steps,
normal evolution, admission, checkpoint durability acknowledgement and handoff.
No descendant, trajectory or cross-backend hash equality is a pass criterion.

Primary outcome: paired difference in last-five-generation mean population
fitness. Also report the paired difference in mean population fitness over all
20 generations, best/median fitness, selected score, food, lifetime, net points,
survival/censoring, completed lives, species, coordinate variance and reproduction.
The independent experimental unit is a seed pair, not an individual genome or
generation. Report every pair and the range of paired effects; use a paired
bootstrap interval and exact paired sign-flip inference as exploratory summaries.
Five pairs cannot establish broad noninferiority or rule out rare pathologies.

Audit all canonical matrices initially and at every generation boundary on both
backends (counterfactual full-i16 quantization on SSE2-evolved populations).
Record scales, RMS, errors, bias/weight scale setters, relative RMSE and outlier
ratios. Count actual pre-clamp overflow separately from ordinary endpoint
occupancy. Preserve canonical F32 recurrent bias statistics separately.
No quantizer, scale domain, bias policy, activation, mutation or crossover change.

Sample eight evenly spaced population slots at four relative-time strata per
generation. Compare pure SSE2/full-i16 forwards using the same recorded input
and common post-publication recurrent state. These bounded counterfactual
one-step diagnostics never feed the world or draw authoritative RNG; they are
not independent-feedback or actual pre-control-state error measurements.
Missing later samples reflect death/delivery availability and are reported.

Record native service time and normal checkpoint/handoff time separately from
cold diagnostics, sampled process RSS, CPU and aggregate steal. Full elapsed
wall time includes diagnostics. Performance across divergent living populations
is workload-dependent; report deliveries and phase costs alongside speed ratios.
Keep the latest ordinary boundary checkpoint for each run, streamed generation
JSONL, source/build identities, logs, receipts and SHA256 manifests in a new
artifact directory. Never overwrite prior studies. Large payloads stay ignored;
commit only tooling, protocol and compact final results/manifests.

Reproduction (in a clean qualification checkout):

```sh
python3 scripts/qualification/run-i16-evolution.py --checkout "$PWD" \
  --artifacts "$PRESERVED_EXACT396_ARTIFACTS" --output "$NEW_STUDY_DIRECTORY"
```

The runner sets `RUSTFLAGS='-C target-cpu=native'`, builds the existing qualifier,
and completes fmt/check/strict all-feature Clippy/default tests/release hook tests
before measurements. It fails on changed starting populations, settings or
source/build contracts; it preserves partial results on failure. Any numerical
or evolutionary pathology is documented before considering a separate follow-up.
