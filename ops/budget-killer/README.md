# Budget kill-switch

## Primary chain (works, tested end-to-end 2026-10-25)

Billing budget `1b22df4f-2d20-4eee-b62d-c4c48bc5968b` (HKD 0.01,
thresholds 50/90/100%) → Pub/Sub topic `budget-alerts` → Cloud
Function `kill-on-budget-alert` (`main.py`) → stops VM `perihel-sync`.

## Monitoring fallback (issue #3)

If the budget's own Pub/Sub notification ever fails, a Cloud
Monitoring alert policy on `billing.googleapis.com/budget/actual_usage`
fires the same Pub/Sub notification channel
(`projects/perihel-tototofu/notificationChannels/8771143626380078324`)
when actual spend passes HK$0.005, and the function stops the VM
from there.

- `budget-fallback-alert.json` — the alert policy as the Monitoring
  API expects it.
- `create-fallback-alert.py` — creates it from a machine with gcloud
  logged in (the Windows machine is fine):

  ```powershell
  python ops/budget-killer/create-fallback-alert.py
  ```

  The script first checks the metric descriptor exists. It only
  appears once the billing account has generated billable usage, so
  until first spend it prints "not registered yet" and creates
  nothing — re-run it after first spend. It also verifies the
  descriptor's monitored resource type is `billing_account` (the
  filter in the JSON assumes that) and skips creation if a policy
  with the same display name already exists, so it is safe to run
  daily.

**It also creates itself.** `tick.py` runs once a minute on the VM, and
`server/sync-server/budget_fallback.py` checks once a day whether the
metric descriptor exists; the first day after billable usage starts it
creates the policy through the VM's service account (metadata token,
no secret) and pushes a notification. So the fallback becomes live
without anyone remembering to run the script. Run
`python3 /opt/perihel-sync/budget_fallback.py` on the VM to see the
current state (`{'state': 'no_metric'}` until first spend).

Creating the policy needs the Monitoring API enabled and a role that
can create alert policies (e.g. `roles/monitoring.alertPolicyEditor`).
The VM's service account already has `roles/editor` and the
`monitoring.write` scope, which is enough.
