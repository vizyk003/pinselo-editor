#!/usr/bin/env bash
# Install fail2ban and CrowdSec on the Oracle server (docs/deploy-oracle.md). Safe to run again. Run it over SSH as
# the deploy user from ~/opencanvas/security, after ./update.sh has started the gateway.
#   ./harden.sh [your IPv4 address or CIDR, never banned; default: this SSH session's address]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
log=$(dirname "$here")/logs/caddy/access.log

session=${SSH_CLIENT:-}
admin=${1:-${session%% *}}
[[ "$admin" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}(/[0-9]{1,2})?$ ]] || { echo "harden: pass your IPv4 address, got '$admin'" >&2; exit 1; }
sudo iptables -nL DOCKER-USER >/dev/null 2>&1 || { echo "harden: no DOCKER-USER chain, is Docker running?" >&2; exit 1; }
[ -d "$(dirname "$log")" ] || { echo "harden: $(dirname "$log") is missing; deploy the current compose.yml and Caddyfile first" >&2; exit 1; }
# Caddy creates the file on the first request, and fail2ban won't start a jail without it.
sudo touch "$log"

# Both firewall clients need Docker's chain, so they start after Docker at boot.
after_docker() {
  sudo mkdir -p "/etc/systemd/system/$1.service.d"
  printf '[Unit]\nAfter=docker.service\nWants=docker.service\n' | sudo tee "/etc/systemd/system/$1.service.d/after-docker.conf" >/dev/null
}

# ------------------------------------------------------------------------------------------------------- fail2ban
sudo apt-get install -y fail2ban python3-systemd
sudo install -m 644 "$here"/fail2ban/filter.d/*.conf /etc/fail2ban/filter.d/
sed -e "s|@IGNORE@|127.0.0.1/8 ::1 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 $admin|" -e "s|@LOG@|$log|" "$here/fail2ban/pinselo.local" |
  sudo tee /etc/fail2ban/jail.d/pinselo.local >/dev/null
sudo fail2ban-client -t
after_docker fail2ban

# ------------------------------------------------------------------------------------------------------- CrowdSec
command -v crowdsec >/dev/null || curl -fsSL https://install.crowdsec.net | sudo sh
sudo apt-get install -y crowdsec crowdsec-firewall-bouncer-iptables
sudo cscli hub update
sudo cscli collections install crowdsecurity/caddy crowdsecurity/base-http-scenarios crowdsecurity/http-cve
# Counts distinct non-static URLs, which a normal editing session reaches. The rate limits cover real floods.
sudo cscli scenarios remove crowdsecurity/http-crawl-non_statics --force || true

sudo mkdir -p /etc/crowdsec/acquis.d
printf 'filenames:\n  - %s\nlabels:\n  type: caddy\n' "$log" | sudo tee /etc/crowdsec/acquis.d/pinselo-caddy.yaml >/dev/null
sudo install -m 644 "$here/crowdsec/pinselo-connector-whitelist.yaml" /etc/crowdsec/parsers/s02-enrich/
printf 'name: pinselo/admin-whitelist\ndescription: "The operator"\nwhitelist:\n  reason: "operator address"\n  %s:\n    - "%s"\n' \
  "$([[ "$admin" == */* ]] && echo cidr || echo ip)" "$admin" | sudo tee /etc/crowdsec/parsers/s02-enrich/pinselo-admin-whitelist.yaml >/dev/null
printf 'iptables_chains:\n  - INPUT\n  - DOCKER-USER\ndisable_ipv6: true\n' |
  sudo tee /etc/crowdsec/bouncers/crowdsec-firewall-bouncer.yaml.local >/dev/null
sudo crowdsec -t
after_docker crowdsec-firewall-bouncer

# ---------------------------------------------------------------------------------------------------------- start
sudo systemctl daemon-reload
sudo systemctl enable fail2ban crowdsec crowdsec-firewall-bouncer
sudo systemctl restart fail2ban crowdsec crowdsec-firewall-bouncer
sleep 5
sudo fail2ban-client status
sudo cscli metrics show acquisition
sudo iptables -S DOCKER-USER
echo "harden: done. Never banned: $admin"
