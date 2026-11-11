"""Create the Monitoring fallback alert as soon as it is possible.

The primary kill chain (budget -> Pub/Sub -> Cloud Function -> VM stop) needs
no help. The fallback is a Cloud Monitoring alert policy on
`billing.googleapis.com/budget/actual_usage`, but that metric descriptor only
exists after the billing account has generated its first billable usage, so
the policy cannot be created before then.

tick.py calls ensure_policy() once a day from the VM. The VM's service
account already carries the monitoring.write scope and roles/editor, so the
token comes from the metadata server: no key file, no secret in the repo.
Until the metric appears this is a single GET that returns nothing; after
first spend the next tick creates the policy, prints it, and notifies.

Policy: mirrors ops/budget-killer/budget-fallback-alert.json (fires the same
Pub/Sub channel as the primary chain when actual spend passes HK$0.005).
"""
import json
import os
import urllib.error
import urllib.request

PROJECT = os.environ.get("PERIHEL_GCP_PROJECT", "perihel-tototofu")
BUDGET_ID = os.environ.get("PERIHEL_BUDGET_ID",
                           "1b22df4f-2d20-4eee-b62d-c4c48bc5968b")
CHANNEL = ("projects/perihel-tototofu/notificationChannels/"
           "8771143626380078324")
METRIC_TYPE = "billing.googleapis.com/budget/actual_usage"
TIMEOUT_S = 30

METADATA_TOKEN_URL = ("http://metadata.google.internal/computeMetadata/v1/"
                      "instance/service-accounts/default/token")

POLICY = {
    "displayName": "Perihel budget fallback - actual spend over HK$0.005",
    "combiner": "AND",
    "conditions": [{
        "displayName": "Budget actual usage over HK$0.005",
        "conditionThreshold": {
            "filter": (f'metric.type="{METRIC_TYPE}" '
                       f'AND resource.type="billing_account" '
                       f'AND metric.label.budget_id="{BUDGET_ID}"'),
            "comparison": "COMPARISON_GT",
            "thresholdValue": 0.005,
            "duration": "0s",
            "aggregations": [{
                "alignmentPeriod": "3600s",
                "perSeriesAligner": "ALIGN_MAX",
            }],
        },
    }],
    "notificationChannels": [CHANNEL],
    "alertStrategy": {"autoClose": "3600s"},
    "enabled": True,
}


def _token() -> str:
    req = urllib.request.Request(METADATA_TOKEN_URL,
                                 headers={"Metadata-Flavor": "Google"})
    with urllib.request.urlopen(req, timeout=TIMEOUT_S) as r:
        return json.loads(r.read().decode())["access_token"]


def _api(token: str, method: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"https://monitoring.googleapis.com/v3/projects/{PROJECT}{path}",
        data=data, method=method,
        headers={"Authorization": f"Bearer {token}",
                 "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_S) as r:
            return json.loads(r.read().decode() or "{}")
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")[:300]
        raise RuntimeError(f"Monitoring API {method} {path} failed "
                           f"({e.code}): {detail}") from None


def ensure_policy(token: str | None = None) -> dict:
    """Returns {"state": "no_metric" | "exists" | "created", "name": str}.
    "no_metric" is the normal state until the first billable usage."""
    token = token or _token()
    descriptors = _api(
        token, "GET",
        f"/metricDescriptors?filter=metric.type%3D%22{METRIC_TYPE}%22",
    ).get("metricDescriptors", [])
    if not descriptors:
        return {"state": "no_metric", "name": ""}

    for policy in _api(token, "GET", "/alertPolicies").get("alertPolicies", []):
        if policy.get("displayName") == POLICY["displayName"]:
            return {"state": "exists", "name": policy["name"]}

    created = _api(token, "POST", "/alertPolicies", POLICY)
    return {"state": "created", "name": created.get("name", "")}


if __name__ == "__main__":
    print(ensure_policy())
