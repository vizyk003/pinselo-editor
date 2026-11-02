# Deploying on Oracle Cloud's free server

Pinselo on one **Oracle Cloud "Always Free" ARM server**, with **Supabase** for the database and file storage.
The server runs the Rust services and the Caddy gateway from prebuilt ARM64 images; it never compiles anything.

| Part | Where | Cost |
|---|---|---|
| api, collab, render, worker, gateway | Oracle Always Free Ampere A1 VM (2 OCPU, 12 GB) | free |
| Database, file storage | Supabase free plan ([supabase.md](supabase.md)) | free |
| Images | GitHub container registry, built by `.github/workflows/images.yml` | free (plan minutes) |
| A domain name | any registrar | about €1–12 a year |
| Email (sign-in links, invites) | an SMTP provider with a free tier | free |

Files: [`deploy/oracle/compose.yml`](../deploy/oracle/compose.yml), [`Caddyfile`](../deploy/oracle/Caddyfile), [`env.example`](../deploy/oracle/env.example),
[`update.sh`](../deploy/oracle/update.sh).

**Know the limits.** Oracle may stop a free server that stays nearly idle (CPU, network and memory all under about
15% for a week); upgrading the account to Pay As You Go (step 1) avoids that while Always Free resources stay free.
Free ARM capacity is sometimes unavailable in busy regions. Oracle has changed the free allowance before. For a
launch with paying users, the same files run on any small ARM or x86 server (build x86 images then).

## 1. Accounts (yours to create)

1. **Oracle Cloud** — <https://www.oracle.com/cloud/free/>. A card is needed for identity verification.
   - The **home region** cannot be changed later. Pick one near your users; if the server step reports "out of
     capacity", a less busy region helps.
   - Recommended: Billing → **Upgrade to Pay As You Go**. Always Free resources stay free, and idle servers are no
     longer reclaimed. Then set a budget alert (Billing → Budgets, e.g. 1 USD) so nothing is ever charged unnoticed.
2. **A domain** for the app. Uploads are served from the same host: files are sniffed, SVGs are sanitised and
   rasterised, and responses carry `nosniff` and a sandboxing CSP. A separate uploads domain is the stronger setup
   (`deploy/docker-compose.yml`), and Supabase Auth should stay off without one (it keeps tokens in browser storage).
3. **Email sending** — any SMTP provider (Resend, Brevo, Postmark, …). You need its SMTP URL and a verified sender.
4. **Supabase** — the project you already have. Follow [supabase.md](supabase.md) sections 1–3: the session pooler
   URL, the CA certificate, a private bucket and a new S3 key pair.

## 2. The server

Oracle Console → Compute → Instances → **Create instance**:

| Setting | Value |
|---|---|
| Image | Canonical Ubuntu 24.04 (the aarch64 build is picked automatically for Ampere) |
| Shape | Ampere → `VM.Standard.A1.Flex`, **2 OCPU, 12 GB** memory |
| Networking | Create a new VCN with a public subnet; **assign a public IPv4 address** |
| SSH keys | Upload your public key (`~/.ssh/id_ed25519.pub`; create one with `ssh-keygen -t ed25519`) |
| Boot volume | 100 GB (Always Free covers up to 200 GB in total) |

"Out of capacity" means no free ARM machines are left in that availability domain right now: try another
availability domain or retry later.

**Open the web ports.** Instance → Subnet → Security list → *Add ingress rules*, source `0.0.0.0/0`:
TCP `80`, TCP `443`, UDP `443` (HTTP/3).

## 3. Prepare the server

```bash
ssh ubuntu@<public IP>

# Docker (the official install script) and the compose plugin.
curl -fsSL https://get.docker.com | sudo sh
sudo usermod -aG docker ubuntu && exit     # log in again so the group applies
ssh ubuntu@<public IP>

# Oracle's Ubuntu image rejects incoming traffic in iptables, besides the security list.
sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 80 -j ACCEPT
sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 443 -j ACCEPT
sudo iptables -I INPUT 6 -m state --state NEW -p udp --dport 443 -j ACCEPT
sudo netfilter-persistent save

# Automatic security updates (on by default on Ubuntu; this makes sure).
sudo apt-get install -y unattended-upgrades

mkdir -p ~/opencanvas
```

**Pull access to the images.** If your fork is private, its images are too. On GitHub: Settings →
Developer settings → Personal access tokens → *Tokens (classic)* → a token with only `read:packages`. Then:

```bash
docker login ghcr.io -u <your GitHub username>     # paste the token as the password
```

## 4. Build the images

GitHub → the repository → **Actions → images → Run workflow**. The first build takes up to about an hour (later
builds reuse the cache). It publishes `ghcr.io/<owner>/pinselo-runtime` and `…-gateway`, tagged `latest`, the
commit and the version tag. Pushing a tag `v*` builds too.

## 5. Settings and first start

From your computer, in the repository:

```bash
scp deploy/oracle/compose.yml deploy/oracle/Caddyfile deploy/oracle/update.sh deploy/oracle/env.example ubuntu@<public IP>:opencanvas/
scp /path/to/supabase-ca.crt ubuntu@<public IP>:opencanvas/supabase-ca.crt
```

On the server:

```bash
cd ~/opencanvas
cp env.example .env && chmod 600 .env
nano .env            # fill in every value (comments say where each comes from)
chmod +x update.sh
```

**DNS.** At your registrar, an `A` record for the app host, pointing at the server's public IP (on Cloudflare: DNS only,
not proxied). Wait until `dig +short <app host>` shows the IP.

```bash
./update.sh          # pulls the images, runs the migrations, starts everything
```

Caddy gets the HTTPS certificates on the first requests. Open `https://<app host>` and sign up; add your email to
`OC_STAFF_EMAILS` for the staff console. The template library is seeded separately (`pnpm seed` from a checkout with
the same Supabase settings, see [supabase.md](supabase.md)).

## 6. Updating

1. GitHub → Actions → **images → Run workflow**, choosing what to build: `both`, `frontend` or `backend` (or push a
   `v*` tag, which builds both).
2. On the server: `cd ~/opencanvas && ./update.sh`, or `./update.sh web` after a frontend build.

`update.sh` runs the migrations before the services restart and removes old images so the disk does not fill up.
`update.sh web` only replaces the gateway, so the backend keeps running. Use the full update for anything else:

- A frontend that needs a new API: deploy the backend first.
- Migrations only run with the full update.
- Fonts, templates and other fixtures are in the runtime image, so they need a backend build.

To go back to an earlier build, set `OC_RUNTIME_IMAGE` and `OC_GATEWAY_IMAGE` in `.env` to the commit tags
(`ghcr.io/<owner>/pinselo-runtime:<commit sha>`) and run `./update.sh` again. A commit tag only exists for the images
that run built, so a frontend-only run has no runtime image under its commit.

## Building on the server

When GitHub Actions isn't available, build the images on the server itself, from your computer:

```bash
deploy/oracle/build-on-server.sh both --deploy     # or frontend, or backend; without --deploy it only builds
```

It sends the committed `HEAD`, adds swap the first time, limits the builder to 1.5 CPUs and 8 GB so the site keeps running, and tags images with the commit. The cache stays on the server, so later builds are faster. To roll back, set `OC_RUNTIME_IMAGE` and `OC_GATEWAY_IMAGE` in `.env` to an older commit's tag and run `./update.sh`.

## Banning abusive clients

The API's own limits answer 429 (10 requests a minute on sign-in routes, 600 on most others). fail2ban and CrowdSec run on the host and drop clients at the firewall when they keep going:

| Tool | Watches | Bans |
|---|---|---|
| fail2ban `sshd` | SSH log | 5 failures in 10 minutes: 1 hour |
| fail2ban `pinselo-auth` | Gateway log: failed sign-ins and 429s on sign-in and account routes | 15 in 10 minutes: 1 hour |
| fail2ban `pinselo-429` | Gateway log: 429s on other routes | 120 in 5 minutes: 1 hour |
| CrowdSec | SSH and gateway logs, for scanners, known CVE probes and brute force | 4 hours |
| CrowdSec community blocklist | Addresses reported by other CrowdSec users | While listed |

fail2ban's ban time grows for repeat offenders, up to 7 days. `/mcp`, `/v1`, `/api/oauth` and the OAuth discovery documents never count towards a ban, because AI connector traffic comes from addresses that providers share between users. CrowdSec reports the addresses it bans to its central service, which is what pays for the blocklist.

Install from your computer, then on the server:

```bash
scp -r deploy/oracle/compose.yml deploy/oracle/Caddyfile deploy/oracle/security ubuntu@<public IP>:opencanvas/
ssh ubuntu@<public IP>
cd ~/opencanvas && docker compose up -d --no-deps gateway
security/harden.sh     # or: security/harden.sh <your IP or CIDR>
```

Your current address is never banned by the rules above. If it changes, run `harden.sh` again with the new one. **Keep this SSH session open** and check that a new one connects before you close it. If you are locked out anyway, use the instance's console connection in the Oracle Console.

The gateway log is at `~/opencanvas/logs/caddy/access.log` and holds client IP addresses for up to 7 days. Secret query parameters and the `Referer` header are removed. A token in a URL path, such as a share link, stays in the log.

```bash
sudo fail2ban-client status pinselo-auth        # who is banned
sudo fail2ban-client unban <IP>
sudo cscli decisions list                       # the same for CrowdSec, without the blocklist
sudo cscli decisions delete --ip <IP>
sudo cscli alerts list
```

Bans cover the whole address, so one abusive client can also block others behind the same office or mobile network. If that happens, unban the address and raise `maxretry` in `/etc/fail2ban/jail.d/pinselo.local`.

## Backups and monitoring

`backup.sh` dumps the database to `~/opencanvas/backups`, keeps the last 7, and copies the Supabase file bucket there. The live data is in Supabase, so this copy is on a different provider. With `OC_BACKUP_S3_*` set in `.env` it also copies everything to a bucket off this server, which protects against losing Supabase and this server together. The backups hold user data; `backups/` is private to this account.

```bash
crontab -e   # 17 3 * * * cd ~/opencanvas && ./backup.sh >> backups/backup.log 2>&1
```

Restore into a throwaway database to test:

```bash
docker run -d --name restore-test -e POSTGRES_PASSWORD=test postgres:17
docker exec -i restore-test sh -c 'until pg_isready -q; do sleep 1; done; pg_restore -U postgres -d postgres --no-owner' < backups/db-<stamp>.dump
docker rm -f restore-test
```

`.github/workflows/uptime.yml` checks `/api/status` hourly and fails, which emails you, when the site is down. For faster alerts, add a free external monitor on `https://example.com/api/status`.

## Troubleshooting

| Symptom | Cause |
|---|---|
| The site does not load at all | Ports: the security list (step 2) **and** iptables (step 3). Check `sudo iptables -L INPUT -n --line-numbers`. |
| Certificate errors on the first visit | DNS not pointing at the server yet, or ports 80/443 blocked; Caddy retries on its own. `docker compose logs gateway`. |
| `denied` when pulling images | `docker login ghcr.io` missing or the token lacks `read:packages`. |
| `Max client connections reached` | The services' pools exceed Supabase's pool size: lower `OC_DATABASE_POOL_MAX`. |
| `self-signed certificate in certificate chain` | `supabase-ca.crt` missing or wrong (it is mounted read-only into every service). |
| A service keeps restarting | `docker compose logs <service>`; most often a setting in `.env`. |
| The server was stopped by Oracle | It was idle for a week; start it in the console, and upgrade to Pay As You Go (step 1). |

Logs: `docker compose logs -f api` (or `collab`, `render`, `worker`, `gateway`). Status: `docker compose ps`.
