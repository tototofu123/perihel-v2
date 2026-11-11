"""Create the Monitoring fallback alert for the Perihel budget kill-switch.

The primary kill chain is budget -> Pub/Sub topic `budget-alerts` ->
Cloud Function `kill-on-budget-alert` -> VM stop, and it already works.
This script adds the fallback: a Cloud Monitoring alert policy on the
billing metric `billing.googleapis.com/budget/actual_usage` that fires
the same Pub/Sub notification channel when actual spend passes
HK$0.005, so the VM still gets stopped if the budget's own Pub/Sub
notification ever fails.

Why a script instead of a one-off console click: the metric descriptor
only exists once the billing account has generated billable usage.
Until first spend, the Monitoring API returns an empty descriptor
list and policy creation fails. This script checks for the descriptor
first and exits cleanly ("not registered yet") so it is safe to
re-run daily until it succeeds.

Run on a machine with gcloud logged in as a billing-account admin
(the Windows machine is fine):

    python ops/budget-killer/create-fallback-alert.py

Needs the Monitoring API enabled and a role that can create alert
policies (e.g. roles/monitoring.alertPolicyEditor) on the project.
Idempotent: if a policy with the same display name already exists,
it prints the existing policy name and stops.
"""

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

PROJECT = "perihel-tototofu"
POLICY_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                           "budget-fallback-alert.json")
METRIC_TYPE = "billing.googleapis.com/budget/actual_usage"
EXPECTED_RESOURCE_TYPE = "billing_account"


def _token() -> str:
    try:
        out = subprocess.run(
            ["gcloud", "auth", "print-access-token"],
            capture_output=True, text=True, timeout=60, check=True,
        )
    except FileNotFoundError:
        print("gcloud is not installed or not on PATH.", file=sys.stderr)
        sys.exit(2)
    except subprocess.CalledProcessError as e:
        print(f"gcloud auth failed: {e.stderr.strip()}", file=sys.stderr)
        sys.exit(2)
    return out.stdout.strip()


def _api(token: str, method: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"https://monitoring.googleapis.com/v3/projects/{PROJECT}{path}",
        data=data, method=method,
        headers={"Authorization": f"Bearer {token}",
                 "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        raise SystemExit(f"Monitoring API {method} {path} failed "
                         f"({e.code}): {detail}")


def main() -> int:
    with open(POLICY_FILE, encoding="utf-8") as f:
        policy = json.load(f)

    token = _token()

    descriptors = _api(
        token, "GET",
        f"/metricDescriptors?filter=metric.type%3D%22{METRIC_TYPE}%22",
    ).get("metricDescriptors", [])
    if not descriptors:
        print(f"Metric {METRIC_TYPE} is not registered yet "
              "(no billable usage has been generated). Nothing created; "
              "run this script again after first spend.")
        return 0

    resource_types = descriptors[0].get("monitoredResourceTypes", [])
    if EXPECTED_RESOURCE_TYPE not in resource_types:
        print(f"Metric descriptor uses monitored resource types "
              f"{resource_types}, not {EXPECTED_RESOURCE_TYPE!r}. "
              f"Fix the filter in {POLICY_FILE} before creating the "
              f"policy.", file=sys.stderr)
        return 1

    policies = _api(token, "GET", "/alertPolicies").get("alertPolicies", [])
    for p in policies:
        if p.get("displayName") == policy["displayName"]:
            print(f"Fallback alert already exists: {p['name']}")
            return 0

    created = _api(token, "POST", "/alertPolicies", policy)
    print(f"Fallback alert created: {created['name']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
