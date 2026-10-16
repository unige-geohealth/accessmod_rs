# Benchmarks

accessmod-rs against AccessMod 5 (`fredmoser/accessmod:5.9.2-beta.6`), one
folder per module. Each folder holds the script (`run.sh`), the part run in
the AccessMod 5 image (`am5.sh`), what is measured and how to read it
(`README.md`), and the generated `report.md` (machine, versions, command,
median of `REPEAT` runs). Figures below come from those reports
(2026-10-16, Apple M1 Pro, 10 CPUs, 32 GB; AccessMod 5 in its Docker VM).

```sh
sh benchmark/travel-time/run.sh 1000 2000 4000
sh benchmark/referral/run.sh 1000 20 2000 50 4000 100
sh benchmark/geographic-coverage/run.sh 1000 20 2000 50 4000 100
sh benchmark/scaling-up/run.sh
```

Needs Docker, the AccessMod 5 image and the oracle exports
(`sh scripts/oracle/export.sh`) for the demo rows. Shared parts:
[`lib.sh`](lib.sh), [`scenario.txt`](scenario.txt), [`am5/`](am5).

## Demo project, full analysis

What a user waits for on the AccessMod 5 demo (227 × 196 cells,
10 facilities): AccessMod 5 replay (loading `global.R`, about 5.4 s, not
included) against the accessmod-rs command, inputs to outputs.

| Module | AccessMod 5 replay | accessmod-rs |
| --- | ---: | ---: |
| [Travel time](travel-time/report.md), 120 min | 1.42 s | 0.24 s |
| [Referral](referral/report.md), 90 pairs | 11.67 s | 0.23 s |
| [Geographic coverage](geographic-coverage/report.md), with catchments | 14.55 s | 0.40 s |
| [Scaling up](scaling-up/report.md), 10 facilities added | 39.39 s | 0.50 s |

## Synthetic landscapes, 16 M cells

AccessMod 5 engine only (`r.walk.accessmod`, data already imported): a
lower bound of its analysis. accessmod-rs: the whole command.

| Module | AccessMod 5 engine | accessmod-rs | Peak RSS AM5 → rs |
| --- | ---: | ---: | ---: |
| [Travel time](travel-time/report.md), 10 facilities | 15.77 s | 16.10 s (solve 14.65 s) | 2 059 → 726 MB |
| [Referral](referral/report.md), 100 facilities, 9 900 pairs | ≥ 35.74 s | 13.51 s | — → 673 MB |
| [Geographic coverage](geographic-coverage/report.md), 100 facilities | ≥ 133.89 s | 19.67 s | — → 546 MB |

Results agree with AccessMod 5: travel times to 7e-16 relative with the
same reachable cells here, and the oracle tests on the demo for every
module (`docs/reference-accessmod5.md`).

## Other studies

- [`solver/`](solver/README.md): tile size and resident tiles, the reason
  for the default tile of 64.
