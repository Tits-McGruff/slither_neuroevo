# PPO learning qualification

Status: empirical qualification to verify; the PPO reward and action-policy implementation was completed and merged through Slither PR #17 and PyRL PR #12.

The original [repair plan](archive/ppo-reward-action-policy-repair-plan.md), especially sections 25 and 26, defines the experimental protocol. Use three fresh training seeds, one million admitted PPO decisions per seed, and ten common evaluation scenarios. Compare trained candidates with their untrained policies and the straight/full-lock reference controls.

Qualification requires improved repaired return for at least two of three seeds, improvement in food consumption or retained growth, useful observation-dependent navigation, and healthy numerical and collection-integrity diagnostics. Report negative or inconclusive results without treating them as an implementation regression.

Do not repeat the completed reward-contract or action-policy repair unless new evidence identifies a concrete defect.
