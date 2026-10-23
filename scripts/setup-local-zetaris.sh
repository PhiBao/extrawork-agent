#!/usr/bin/env bash
# Prepare the supported local Zetaris Docker installation for ExtraWork.
#
# You must supply the licensed platform bundle and registry access:
#   1. Sign in at https://cloud.enterprise.zetaris.com/dashboard/download
#   2. Download zetaris-platform.zip and run the registry login shown there
#   3. Place the zip at vendor/zetaris-platform.zip (or pass its path)
#
# This script does not download or license Zetaris. It configures the supplied
# bundle, starts it, and wires ExtraWork to the local Gateway.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="${1:-$ROOT/vendor/zetaris-platform.zip}"
TARGET="${ZETARIS_LOCAL_DIR:-$HOME/zetaris-local}"
EMAIL="${ZETARIS_ADMIN_EMAIL:-admin@extrawork.dev}"
ORG="${ZETARIS_ADMIN_ORG:-ExtraWork}"

if [ ! -e "$BUNDLE" ]; then
  cat <<EOF
No Zetaris bundle found at:
  $BUNDLE

Download it from the Zetaris Cloud account that created the AWS cluster:
  https://cloud.enterprise.zetaris.com/dashboard/download

Then place zetaris-platform.zip at vendor/zetaris-platform.zip and run:
  ./scripts/setup-local-zetaris.sh

The bundle is licensed and not included in this repository.
EOF
  exit 1
fi

if [ -d "$BUNDLE" ]; then
  mkdir -p "$TARGET"
  cp -a "$BUNDLE"/. "$TARGET"/
elif [ "${BUNDLE##*.}" = "zip" ]; then
  mkdir -p "$TARGET"
  unzip -o -q "$BUNDLE" -d "$TARGET"
  if [ -d "$TARGET/zetaris-platform" ] && [ -f "$TARGET/zetaris-platform/docker-compose.yml" ]; then
    TARGET="$TARGET/zetaris-platform"
  fi
else
  echo "Expected a .zip or an extracted platform directory."; exit 1
fi

cd "$TARGET"
[ -f docker-compose.yml ] || { echo "docker-compose.yml not found in $TARGET"; exit 1; }
[ -f .env ] || { echo ".env not found; use the .env supplied with the bundle."; exit 1; }

python3 - "$TARGET" "$EMAIL" "$ORG" <<'PY'
import secrets, string, sys
from pathlib import Path
target = Path(sys.argv[1]); email = sys.argv[2]; org = sys.argv[3]
env = target / ".env"
password_file = target / ".admin-password"
if password_file.exists():
    password = password_file.read_text().strip()
else:
    alphabet = string.ascii_letters + string.digits + "$@!%*?&"
    while True:
        password = "".join(secrets.choice(alphabet) for _ in range(24))
        if (any(c.islower() for c in password) and any(c.isupper() for c in password)
                and any(c.isdigit() for c in password)
                and any(c in "$@!%*?&" for c in password)):
            break
    password_file.write_text(password + "\n")
    password_file.chmod(0o600)

updates = {
    "ZETARIS_ADMIN_EMAIL": email,
    "ZETARIS_ADMIN_PASSWORD": password,
    "ZETARIS_ADMIN_ORG": org,
    "ZSPARK_DRIVER_MEM": "2g",
    "ZSPARK_DRIVER_CORES": "2",
    "ZSPARK_MASTER_URL": "local[2]",
    "ZETARIS_API_MAX_HEAP": "1g",
    "UI_PORT": "3000",
    "ALLOWED_ORIGINS": "http://localhost:3000",
    "GUI_URL": "http://localhost:3000",
    "COMPOSE_PROFILES": "",
    "ZETARIS_SEED_TPCH": "true",
}
lines = env.read_text().splitlines()
seen = set()
out = []
for line in lines:
    if "=" in line and not line.lstrip().startswith("#"):
        key = line.split("=", 1)[0].strip()
        if key in updates:
            out.append(f"{key}={updates[key]}")
            seen.add(key)
            continue
    out.append(line)
for key, value in updates.items():
    if key not in seen:
        out.append(f"{key}={value}")
env.write_text("\n".join(out) + "\n")
env.chmod(0o600)
print(f"configured {env}")
PY

echo "== validating compose config (no secrets printed) =="
docker compose config --quiet
echo "== preflight =="
if [ -x ./preflight.sh ]; then ./preflight.sh || { echo "Resolve preflight failures, then rerun."; exit 1; }; fi
echo "== pulling images =="
docker compose pull
echo "== starting =="
docker compose up -d
echo "== waiting for the Gateway =="
for _ in $(seq 1 60); do
  if curl -fsS -o /dev/null http://127.0.0.1:3000 2>/dev/null; then break; fi
  sleep 15
done
docker compose ps --all
if [ -x ./verify.sh ]; then ./verify.sh || true; fi

# Wire ExtraWork without printing the password.
python3 - "$ROOT" "$TARGET" "$EMAIL" <<'PY'
import sys
from pathlib import Path
root = Path(sys.argv[1]); target = Path(sys.argv[2]); email = sys.argv[3]
password = (target / ".admin-password").read_text().strip()
env = root / ".env"
lines = env.read_text().splitlines() if env.exists() else []
values = {
    "ZETARIS_API_STYLE": "gateway",
    "ZETARIS_BASE_URL": "http://127.0.0.1:3000",
    "ZETARIS_USERNAME": email,
    "ZETARIS_PASSWORD": password,
}
seen = set(); out = []
for line in lines:
    if "=" in line and not line.lstrip().startswith("#"):
        key = line.split("=", 1)[0].strip()
        if key in values:
            out.append(f"{key}={values[key]}"); seen.add(key); continue
    out.append(line)
for key, value in values.items():
    if key not in seen:
        out.append(f"{key}={value}")
env.write_text("\n".join(out) + "\n")
env.chmod(0o600)
print("wrote Zetaris settings to .env (password kept in the local file only)")
PY

cat <<EOF

Local Zetaris is starting. Check:
  cd $TARGET && docker compose ps --all
  http://localhost:3000

The admin password is in:
  $TARGET/.admin-password

ExtraWork now has Gateway settings in .env. Restart it with:
  ./scripts/demo.sh

Next: register or create the two tables the join expects (project_lines,
evidence_extracts) in the local Zetaris instance, then open the case. The
README's Zetaris section explains the fixed join.
EOF
