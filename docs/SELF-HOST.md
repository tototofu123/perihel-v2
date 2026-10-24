# Self-hosting Perihel sync

Everything runs on **always-free** cloud resources — no paid tiers anywhere.
Total expected bill: $0. The live deployment's private values (URL, token,
GCP project) are in `server/local.env`, which is **gitignored** — this guide
is written so anyone can recreate an equivalent server without them.

## Architecture

```
Android app ─┐
             ├─ HTTPS (Let's Encrypt via Caddy) ──► sync server (Python, stdlib only)
Desktop app ─┘                                        └─ SQLite on the VM
VM timer (10 min) ──► ntfy.sh ──► your phone (resurfacing pushes)
```

- The VM is a **relay only**. No LLM, no weights — 1 GB RAM can't host models.
  LLM answers come from a free API (NVIDIA NIM by default) or the local .gguf.
- Local SQLite on each device stays the source of truth; the server is the
  meeting point. Offline always works.

## 1. GCP project (free tier, charge-proof)

```bash
gcloud projects create perihel-$USER          # any unique id
gcloud config set project perihel-$USER
```

Link a billing account (required for Compute Engine even on free tier) and
**immediately** set a tiny budget so any surprise charge emails you:

```bash
gcloud billing budgets create \
  --billing-account=YOUR_BILLING_ACCOUNT \
  --display-name="perihel 0.01 alert" \
  --budget-amount=0.01HKD --filter-projects="projects/YOUR_PROJECT_NUMBER" \
  --threshold-rule=percent=50 --threshold-rule=percent=90 --threshold-rule=percent=100
```

(GCP has **no hard spend cap** — the budget alerts, it does not block. The
kill switch below is what actually stops the meter.)

## 2. VM

```bash
gcloud services enable compute.googleapis.com
gcloud compute instances create perihel-sync \
  --zone=us-west1-b --machine-type=e2-micro \
  --image-family=debian-12 --image-project=debian-cloud \
  --boot-disk-size=30GB --boot-disk-type=pd-standard --tags=perihel
# e2-micro + us-west1/us-central1/us-east4 + 30GB pd-standard = always-free SKUs
```

Promote the ephemeral IP to static (free for free-tier e2-micro):

```bash
gcloud compute addresses create perihel-ip --region=us-west1 \
  --addresses=$(gcloud compute instances describe perihel-sync \
    --zone=us-west1-b --format="value(networkInterfaces[0].accessConfigs[0].natIP)")
```

## 3. Sync server + Caddy (real TLS, free domain)

On the VM (`gcloud compute ssh perihel-sync --zone=us-west1-b`):

```bash
sudo mkdir -p /opt/perihel-sync && cd /opt/perihel-sync
sudo cp ~/server.py ~/ntfy_push.py .            # from server/sync-server/ in this repo
openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 3650 \
  -nodes -subj "/CN=perihel-sync" -addext "subjectAltName=IP:YOUR_VM_IP"
openssl rand -hex 32 | sudo tee token           # bearer token — the password
openssl rand -hex 4 | sed "s/^/perihel-/" | sudo tee ntfy-topic
sudo chmod 600 key.pem token ntfy-topic
```

Install systemd units (see `server/systemd/`), then Caddy terminates real TLS:

```bash
sudo apt-get install -y caddy
# /etc/caddy/Caddyfile:  YOUR-VM-IP- dashes .sslip.io  →  reverse_proxy 127.0.0.1:8443
```

`sslip.io` is a free wildcard DNS that maps `203-0-113-10.sslip.io`-style
names to the IP, so Let's Encrypt issues a **real trusted certificate** with
zero domain setup. Firewall: open tcp:80+443, nothing else.

## 4. Budget kill-switch (auto-stop if ever charged)

`ops/budget-killer/` is a Cloud Function wired to the budget via Pub/Sub:
any threshold crossing stops the VM. Deploy per the README in that folder.
Also add a monitoring email channel to the budget for the alert email.

## 5. Clients

- **Desktop**: `%APPDATA%/perihel/settings.json` → `sync_enabled: true`,
  `server_url`, `device_token` (= VM token file).
- **Mobile**: `mobile/` — the app asks for server URL + token on first run.
- **Phone pushes**: install the free ntfy app, subscribe to the topic from
  `/opt/perihel-sync/ntfy-topic`.

## Kill switch (if anything ever looks wrong)

```bash
gcloud billing projects unlink perihel-$USER    # stops ALL billable activity
# or just: gcloud compute instances stop perihel-sync --zone=us-west1-b
```
