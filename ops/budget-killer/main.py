"""Budget-killer: stops the perihel-sync VM when a budget threshold fires.

Deliberately uses only urllib + the metadata server — the google-cloud-compute
client library OOM'd the 244 MiB Cloud Run sandbox, and a crashed invocation
makes Pub/Sub redeliver the alert, which would keep stopping a restarted VM.
Dedupes on the Pub/Sub message ID via project metadata, so redelivery of the
same alert is a no-op.
"""
import json
import logging
import os
import urllib.request

import functions_framework

PROJECT = "perihel-tototofu"
ZONE = "us-west1-b"
INSTANCE = "perihel-sync"
METADATA_KEY = "perihel-last-budget-msg"

logging.basicConfig(level=logging.INFO)


def _metadata(path: str) -> str:
    req = urllib.request.Request(
        f"http://metadata.google.internal/computeMetadata/v1/{path}",
        headers={"Metadata-Flavor": "Google"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.read().decode()


def _compute_rest(method: str, path: str, body: dict | None = None) -> dict:
    token = _metadata("instance/service-accounts/default/token")
    url = f"https://compute.googleapis.com/compute/v1/projects/{PROJECT}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"Authorization": f"Bearer {token}",
                                          "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read().decode() or "{}")


def _seen_message(msg_id: str) -> bool:
    desc = _compute_rest("GET", f"?fields=items,name")  # project info
    items = {i["key"]: i["value"] for i in desc.get("items", [])}
    return items.get(METADATA_KEY) == msg_id


def _mark_message(msg_id: str):
    body = _compute_rest("GET", "?fields=items,fingerprint,description")
    items = [i for i in body.get("items", []) if i["key"] != METADATA_KEY]
    items.append({"key": METADATA_KEY, "value": msg_id})
    _compute_rest("POST", "?alt=json", {"items": items,
                                        "fingerprint": body["fingerprint"]})


@functions_framework.cloud_event
def kill_on_budget_alert(cloud_event):
    import base64
    msg = cloud_event.data["message"]
    msg_id = msg.get("messageId", "unknown")
    if _seen_message(msg_id):
        logging.info("message %s already processed — skipping (redelivery)", msg_id)
        return "deduped"

    payload = json.loads(base64.b64decode(msg["data"]))
    cost = float(payload.get("costAmount", 0))
    budget = float(payload.get("budgetAmount", 0))
    threshold = float(payload.get("alertThresholdExceeded", 0) or 0)
    logging.info("budget=%s cost=%.2f budget=%.2f threshold=%.2f",
                 payload.get("budgetDisplayName"), cost, budget, threshold)

    if threshold <= 0 and budget > 0 and cost < budget:
        _mark_message(msg_id)
        return "no-action"

    inst = _compute_rest("GET", f"/zones/{ZONE}/instances/{INSTANCE}")
    if inst.get("status") == "TERMINATED":
        _mark_message(msg_id)
        logging.info("%s already stopped.", INSTANCE)
        return "already-stopped"

    _compute_rest("POST", f"/zones/{ZONE}/instances/{INSTANCE}/stop", {})
    _mark_message(msg_id)
    logging.info("STOPPED %s — budget alert fired (cost %.2f / %.2f %s).",
                 INSTANCE, cost, budget, payload.get("currencyCode", "?"))
    return "vm-stopped"
