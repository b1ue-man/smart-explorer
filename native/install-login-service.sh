#!/usr/bin/env bash
# Optional administrative login installation. Does not call the per-user updater.
# Usage: sudo ./install-login-service.sh install IMAGE EXPECTED_SHA256 VERSION POLICY_JSON [MACHINE_IDENTITY_JSON] [GATEWAY_NETWORK_JSON]
# POLICY_JSON is an explicit administrative approval of machine/peer/users/rights.
# First installation needs a freshly provisioned MachineIdentity JSON, unless
# the private gateway machine-key.json already exists. Public key and machine ID
# must match the explicit administrative policy; no user profile key is imported.
# Usage: sudo ./install-login-service.sh uninstall
set -euo pipefail
[[ ${EUID} == 0 ]] || { echo 'Login service installation requires root.' >&2; exit 1; }
operation=${1:-}
config=/var/lib/smart-explorer-login
state=/var/lib/smart-explorer-login-gateway
root=/usr/libexec/smart-explorer-login
runtime=/run/smart-explorer-login
public_status=/var/lib/smart-explorer-login-status
broker=smart-explorer-login-broker.service
gateway=smart-explorer-login-gateway.service
marker='# Managed by Smart Explorer optional login service v1'
# Refuse symlink ancestry; the install never follows a user-chosen destination.
check_ancestry() {
    local path=$1 current=/ part
    IFS=/ read -r -a parts <<< "${path#/}"
    for part in "${parts[@]}"; do
        current=${current%/}/$part
        if [[ -e $current || -L $current ]]; then
            [[ ! -L $current && -d $current ]] || { echo "Unsafe installation directory: $current" >&2; exit 1; }
            [[ $(stat -c %u "$current") == 0 ]] || { echo "Installation ancestor is not root-owned: $current" >&2; exit 1; }
            local mode; mode=$(stat -c %a "$current")
            (( (8#$mode & 8#022) == 0 )) || { echo "Installation ancestor permits non-root writes: $current" >&2; exit 1; }
        else
            install -d -o root -g root -m 0755 "$current"
        fi
    done
}
managed_unit() {
    [[ ! -e $1 && ! -L $1 ]] && return 0
    [[ -f $1 && ! -L $1 && $(stat -c %u "$1") == 0 && $(head -n 1 "$1") == "$marker" ]] || return 1
    local mode; mode=$(stat -c %a "$1")
    (( (8#$mode & 8#022) == 0 ))
}
stop_managed_units() {
    local unit
    for unit in "$gateway" "$broker"; do
        if [[ $(systemctl show -p LoadState --value "$unit") != not-found ]]; then systemctl stop "$unit" || return 1; fi
    done
}
check_ancestry "$root"
# Install and uninstall share one lock outside the active config directory.
[[ ! -L $root/install.lock ]] || { echo 'Unsafe administrative install lock.' >&2; exit 1; }
if [[ -e $root/install.lock ]]; then
    [[ -f $root/install.lock && $(stat -c %u "$root/install.lock") == 0 && $(stat -c %h "$root/install.lock") == 1 ]] || { echo 'Unsafe administrative install lock owner/kind.' >&2; exit 1; }
fi
umask 077
exec 9>>"$root/install.lock"
flock -n 9 || { echo 'An administrative login installation is already running.' >&2; exit 1; }
for unit in "$broker" "$gateway"; do
    managed_unit "/etc/systemd/system/$unit" || { echo "Refusing to replace unmanaged unit: $unit" >&2; exit 1; }
    fragment=$(systemctl show -p FragmentPath --value "$unit")
    [[ -z $fragment || $fragment == "/etc/systemd/system/$unit" ]] || { echo "Refusing to alter unit from another installation: $unit" >&2; exit 1; }
done
if [[ $operation == uninstall ]]; then
    check_ancestry "$config"
    stop_managed_units
    for unit in "$gateway" "$broker"; do
        [[ ! -f /etc/systemd/system/$unit ]] || systemctl disable "$unit"
    done
    rm -f -- "/etc/systemd/system/$gateway" "/etc/systemd/system/$broker"
    systemctl daemon-reload
    # Policy, machine key, network configuration and generations are preserved.
    echo 'Optional login services removed; administrative grants, network configuration and private machine identity retained.'
    exit 0
fi
[[ $operation == install && $# -ge 5 && $# -le 7 ]] || { echo 'Usage: install IMAGE EXPECTED_SHA256 VERSION POLICY_JSON [MACHINE_IDENTITY_JSON] [GATEWAY_NETWORK_JSON] | uninstall' >&2; exit 2; }
image=$2 expected=${3,,} version=$4 policy=$5 identity=${6:-$state/machine-key.json} network=${7:-$state/gateway-network.json}
[[ $expected =~ ^[0-9a-f]{64}$ && $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'An explicit SHA256 and release version are required.' >&2; exit 2; }
command -v python3 >/dev/null
command -v systemctl >/dev/null
check_ancestry "$root/releases"
generation="$version-${expected:0:16}"
stage=$(mktemp -d "$root/releases/.stage-XXXXXXXX")
rollback_active=0
services_changed=0
new_generation=0
account_created=0
group_created=0
declare -a journal_paths=() journal_copies=() directory_paths=() directory_meta=()
declare -A was_active=() was_enabled=()
snapshot_file() {
    local path=$1 index=${#journal_paths[@]}
    [[ ! -L $path ]] || { echo "Unsafe administrative record: $path" >&2; return 1; }
    journal_paths+=("$path")
    if [[ -e $path ]]; then
        [[ -f $path && $(stat -c %h "$path") == 1 && $(stat -c %s "$path") -le 1048576 ]] || { echo "Unsafe administrative record kind/link/size: $path" >&2; return 1; }
        cp -a -- "$path" "$stage/record-$index"
        journal_copies+=("$stage/record-$index")
    else journal_copies+=(''); fi
}
snapshot_directory() {
    local path=$1 index=${#directory_paths[@]}
    [[ ! -L $path ]] || { echo "Unsafe administrative directory: $path" >&2; return 1; }
    directory_paths+=("$path"); directory_meta+=("$stage/directory-$index.json")
    python3 - "$path" "$stage/directory-$index.json" <<'PYMETA'
import base64, json, os, stat, sys
path, dest = sys.argv[1:]
record = None
if os.path.lexists(path):
    s = os.lstat(path)
    if not stat.S_ISDIR(s.st_mode): raise SystemExit('Administrative destination is not a directory.')
    attrs = {name: base64.b64encode(os.getxattr(path, name)).decode('ascii') for name in os.listxattr(path) if name.startswith('system.posix_acl_')}
    record = dict(uid=s.st_uid, gid=s.st_gid, mode=stat.S_IMODE(s.st_mode), attrs=attrs)
with open(dest, 'x') as f: json.dump(record, f)
PYMETA
}
rollback() {
    if (( services_changed )) && ! stop_managed_units; then
        echo "Unable to stop candidate services; protected rollback evidence retained at $stage." >&2
        return 1
    fi
    local index path unit enabled
    if (( services_changed )); then
        for unit in "$gateway" "$broker"; do
            if [[ -f /etc/systemd/system/$unit ]]; then systemctl disable "$unit" || return 1; fi
        done
    fi
    for ((index=0; index<${#journal_paths[@]}; index++)); do
        path=${journal_paths[index]}
        rm -f -- "$path"
        [[ -z ${journal_copies[index]} ]] || cp -a -- "${journal_copies[index]}" "$path" || return 1
    done
    for ((index=${#directory_paths[@]}-1; index>=0; index--)); do
        python3 - "${directory_paths[index]}" "${directory_meta[index]}" <<'PYRESTORE'
import base64, json, os, sys
path, source = sys.argv[1:]
with open(source) as f: record = json.load(f)
if record is None:
    if os.path.isdir(path): os.rmdir(path)
else:
    if not os.path.isdir(path): os.mkdir(path, 0o700)
    os.chown(path, record['uid'], record['gid']); os.chmod(path, record['mode'])
    for name in os.listxattr(path):
        if name.startswith('system.posix_acl_') and name not in record['attrs']: os.removexattr(path, name)
    for name, value in record['attrs'].items(): os.setxattr(path, name, base64.b64decode(value))
PYRESTORE
        [[ $? == 0 ]] || return 1
    done
    if (( services_changed )); then
        systemctl daemon-reload || return 1
        for unit in "$broker" "$gateway"; do
            enabled=${was_enabled[$unit]}
            case "$enabled" in enabled) systemctl enable "$unit" || return 1;; enabled-runtime) systemctl enable --runtime "$unit" || return 1;; esac
        done
        for unit in "$broker" "$gateway"; do [[ ${was_active[$unit]} != active ]] || systemctl start "$unit" || return 1; done
    fi
    if (( new_generation )); then rm -rf -- "$root/releases/$generation"; fi
    echo 'Administrative login installation failed; original records, ownership, ACLs and service startup/runtime state restored.' >&2
}
cleanup() {
    local status=$? retain=0
    if (( status != 0 && rollback_active != 0 )); then rollback || retain=1; fi
    if (( status != 0 && retain == 0 && account_created )); then
        if getent passwd smart-explorer-login >/dev/null; then userdel smart-explorer-login || retain=1; fi
        if (( retain == 0 && group_created )) && getent group smart-explorer-login >/dev/null; then groupdel smart-explorer-login || retain=1; fi
    fi
    if (( retain == 0 )); then [[ -z ${stage:-} ]] || rm -rf -- "$stage"; fi
    return "$status"
}
trap cleanup EXIT
install -m 0555 -o root -g root "$image" "$stage/smart-explorer-login"
actual=$(sha256sum "$stage/smart-explorer-login"); actual=${actual%% *}
[[ $actual == "$expected" ]] || { echo 'Staged executable SHA256 does not match the approved release.' >&2; exit 1; }
if ! getent passwd smart-explorer-login >/dev/null; then
    account_created=1
    if ! getent group smart-explorer-login >/dev/null; then group_created=1; fi
    useradd --system --user-group --home-dir "$state" --no-create-home --shell "$(command -v nologin)" smart-explorer-login
fi
uid=$(id -u smart-explorer-login) gid=$(id -g smart-explorer-login)
[[ $uid != 0 ]] || { echo 'Gateway system account must be unprivileged.' >&2; exit 1; }
account_shell=$(getent passwd smart-explorer-login | cut -d: -f7)
[[ $account_shell == */nologin || $account_shell == */false ]] || { echo 'Gateway account must not permit interactive login.' >&2; exit 1; }
[[ ! -e $state || $(stat -c %u "$state") == 0 || $(stat -c %u "$state") == "$uid" ]] || { echo 'Gateway state directory has an unexpected owner.' >&2; exit 1; }
python3 - "$policy" "$stage/policy.json" "$uid" "$version" "$generation" "$expected" "$identity" "$stage/machine-key.json" "$network" "$stage/gateway-network.json" <<'PY'
import json, os, sys, urllib.parse
source, dest, uid, version, generation, sha, identity_source, identity_dest, network_source, network_dest = sys.argv[1:]
with open(source, 'rb') as f:
    raw = f.read(1024 * 1024 + 1)
if len(raw) > 1024 * 1024: raise SystemExit('Policy exceeds one MiB.')
p = json.loads(raw)
if p.get('format') != 1 or not p.get('revision', 0) or not p.get('machine_public_key') or not p.get('machine_id'):
    raise SystemExit('Policy requires an explicit machine identity and revision.')
if not p.get('grants') or any(not g.get('target_users') or not g.get('public_key') or not g.get('rights', {}).get('login') for g in p['grants']):
    raise SystemExit('Policy requires explicit peer, target-user and login grants.')
try:
    key_fd = os.open(identity_source, os.O_RDONLY | os.O_NOFOLLOW)
except FileNotFoundError:
    raise SystemExit('First login installation requires an explicit newly provisioned MachineIdentity JSON.')
with os.fdopen(key_fd, 'rb') as f: key_raw = f.read(8193)
if len(key_raw) > 8192: raise SystemExit('Machine identity exceeds its limit.')
k = json.loads(key_raw)
if set(k) != {'format', 'machine_id', 'public_key', 'secret_key'} or k['format'] != 1 or k['machine_id'] != p['machine_id'] or k['public_key'] != p['machine_public_key']:
    raise SystemExit('Private machine identity does not match the administrative policy.')
if len(k['secret_key']) != 64 or any(c not in '0123456789abcdefABCDEF' for c in k['secret_key']):
    raise SystemExit('Machine identity requires its newly generated Iroh private key.')
with open(identity_dest, 'x', encoding='utf8') as f: json.dump(k, f, separators=(',', ':')); f.flush(); os.fsync(f.fileno())
os.chmod(identity_dest, 0o600)
try:
    network_fd = os.open(network_source, os.O_RDONLY | os.O_NOFOLLOW)
except FileNotFoundError:
    raise SystemExit('First installation requires explicit gateway-network.json from the private provision bundle.')
with os.fdopen(network_fd, 'rb') as f: network_raw = f.read(32769)
if len(network_raw) > 32768: raise SystemExit('Gateway network configuration exceeds 32 KiB.')
n = json.loads(network_raw)
if set(n) != {'format', 'use_default_relays', 'relay_urls', 'relay_pins', 'allow_plaintext_relays'} or type(n['format']) is not int or n['format'] != 1:
    raise SystemExit('Gateway network configuration has an unsupported format.')
if type(n['use_default_relays']) is not bool or type(n['allow_plaintext_relays']) is not bool or not isinstance(n['relay_urls'], list) or not isinstance(n['relay_pins'], list):
    raise SystemExit('Gateway network configuration has invalid field types.')
if len(n['relay_urls']) > 16 or len(n['relay_pins']) > 16 or (n['use_default_relays'] and n['relay_urls']):
    raise SystemExit('Gateway network configuration exceeds relay limits.')
for url in n['relay_urls']:
    if not isinstance(url, str) or len(url) > 2048 or any(c in url for c in '\0\r\n'):
        raise SystemExit('Invalid gateway relay URL.')
    u = urllib.parse.urlsplit(url)
    if not u.hostname or u.username is not None or u.password is not None or u.query or u.fragment or u.scheme not in ('https', 'http') or (u.scheme == 'http' and not n['allow_plaintext_relays']):
        raise SystemExit('Gateway relay URL needs HTTPS or explicit plaintext permission and cannot contain credentials, a query or a fragment.')
for pin in n['relay_pins']:
    if not isinstance(pin, list) or len(pin) != 32 or any(type(v) is not int or v < 0 or v > 255 for v in pin):
        raise SystemExit('Gateway relay certificate pin must contain exactly 32 bytes.')
network_json = json.dumps(n, separators=(',', ':'), ensure_ascii=False)
if len(network_json.encode('utf8')) > 32768: raise SystemExit('Canonical gateway network configuration exceeds its byte limit.')
with open(network_dest, 'x', encoding='utf8') as f: f.write(network_json); f.flush(); os.fsync(f.fileno())
os.chmod(network_dest, 0o600)
p['gateway_identity'] = uid
with open(dest, 'x', encoding='utf8') as f: json.dump(p, f, separators=(',', ':')); f.flush(); os.fsync(f.fileno())
with open(os.path.join(os.path.dirname(dest), 'manifest.json'), 'x', encoding='utf8') as f:
    json.dump(dict(format=1, version=version, generation=generation, sha256=sha), f, separators=(',', ':')); f.flush(); os.fsync(f.fileno())
PY
chmod 0640 "$stage/policy.json"
chown root:"$gid" "$stage/policy.json"
chmod 0644 "$stage/manifest.json"
"$stage/smart-explorer-login" --login-install-validate "$version" "$expected" "$stage/policy.json" "$stage/machine-key.json" "$stage/gateway-network.json" "$stage/manifest.json"
# Snapshot the complete affected record set before altering the live installation.
for directory in "$config" "$state" "$runtime" "$public_status"; do snapshot_directory "$directory"; done
for record in policy manifest; do
    for suffix in .json .next.json .previous.json; do snapshot_file "$config/$record$suffix"; done
done
for record in machine-key gateway-network; do
    for suffix in .json .next.json .previous.json; do snapshot_file "$state/$record$suffix"; done
done
snapshot_file "$state/gateway-target.json"
snapshot_file "$public_status/owner.marker"
snapshot_file "$public_status/status.json"
for unit in "$broker" "$gateway"; do
    snapshot_file "/etc/systemd/system/$unit"
    snapshot_file "$config/$unit.previous"
    was_active[$unit]=$(systemctl is-active "$unit" || true)
    case "${was_active[$unit]}" in active|inactive|failed|unknown|'') ;; *) echo 'Managed login services are changing state; no installation changes made.' >&2; exit 1;; esac
    was_enabled[$unit]=$(systemctl is-enabled "$unit" || true)
    case "${was_enabled[$unit]}" in enabled|enabled-runtime|disabled|not-found|'') ;; *) echo 'Unsupported original managed-unit enablement; no changes made.' >&2; exit 1;; esac
done
rollback_active=1
# Complete isolated release generation first; never overwrite an executable in use.
if [[ -e $root/releases/$generation ]]; then
    check_ancestry "$root/releases/$generation"
    [[ ! -L $root/releases/$generation/smart-explorer-login && -f $root/releases/$generation/smart-explorer-login
        && $(stat -c %u "$root/releases/$generation/smart-explorer-login") == 0
        && $(stat -c %h "$root/releases/$generation/smart-explorer-login") == 1
        && $(sha256sum "$root/releases/$generation/smart-explorer-login" | cut -d' ' -f1) == "$expected" ]] || { echo 'Existing protected generation differs.' >&2; exit 1; }
else
    new_generation=1
    install -d -o root -g root -m 0755 "$root/releases/$generation"
    install -o root -g root -m 0555 "$stage/smart-explorer-login" "$root/releases/$generation/smart-explorer-login"
fi
services_changed=1
stop_managed_units
check_ancestry "$config"
check_ancestry "$public_status"
[[ ! -L $state && ! -L $runtime ]] || { echo 'Gateway state/runtime directory must not be a symlink.' >&2; exit 1; }
install -d -o "$uid" -g "$gid" -m 0700 "$state"
install -d -o root -g "$gid" -m 0750 "$runtime"
chmod 0750 "$config"; chown root:"$gid" "$config"
if [[ ! -e $public_status/owner.marker ]]; then install -o root -g root -m 0644 /dev/null "$public_status/owner.marker"; fi
chmod 0755 "$public_status"
# Keep one administrative rollback pair before replacing the manifest last.
for record in policy manifest; do
    rm -f -- "$config/$record.previous.json"
    [[ ! -f $config/$record.json ]] || cp -p -- "$config/$record.json" "$config/$record.previous.json"
done
for unit in "$broker" "$gateway"; do
    rm -f -- "$config/$unit.previous"
    [[ ! -f /etc/systemd/system/$unit ]] || cp -p -- "/etc/systemd/system/$unit" "$config/$unit.previous"
done
for record in machine-key gateway-network; do
    rm -f -- "$state/$record.previous.json"
    [[ ! -f $state/$record.json ]] || cp -p -- "$state/$record.json" "$state/$record.previous.json"
done
for record in machine-key gateway-network; do
    install -o "$uid" -g "$gid" -m 0600 "$stage/$record.json" "$state/$record.next.json"
    mv -fT -- "$state/$record.next.json" "$state/$record.json"
done
install -o root -g "$gid" -m 0640 "$stage/policy.json" "$config/policy.next.json"
install -o root -g "$gid" -m 0640 "$stage/manifest.json" "$config/manifest.next.json"
mv -fT -- "$config/policy.next.json" "$config/policy.json"
mv -fT -- "$config/manifest.next.json" "$config/manifest.json"
image_path="$root/releases/$generation/smart-explorer-login"
cat >"/etc/systemd/system/$broker" <<UNIT
$marker
[Unit]
Description=Smart Explorer local login broker
After=systemd-logind.service
[Service]
Type=notify
NotifyAccess=main
TimeoutStartSec=30
User=root
Group=smart-explorer-login
ExecStart=$image_path --login-broker-service
RuntimeDirectory=smart-explorer-login
RuntimeDirectoryMode=0750
UMask=0027
Restart=on-failure
TimeoutStopSec=15
KillMode=mixed
RestrictAddressFamilies=AF_UNIX
ProtectSystem=strict
ReadWritePaths=/run/smart-explorer-login $public_status
ProtectHome=read-only
[Install]
WantedBy=multi-user.target
UNIT
cat >"/etc/systemd/system/$gateway" <<UNIT
$marker
[Unit]
Description=Smart Explorer unprivileged pre-login network gateway
After=network-online.target $broker
Wants=network-online.target
Requires=$broker
[Service]
Type=notify
NotifyAccess=main
TimeoutStartSec=30
User=smart-explorer-login
Group=smart-explorer-login
ExecStart=$image_path --login-gateway-service
StateDirectory=smart-explorer-login-gateway
StateDirectoryMode=0700
UMask=0077
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=$state
Restart=on-failure
TimeoutStopSec=15
[Install]
WantedBy=multi-user.target
UNIT
chmod 0644 "/etc/systemd/system/$broker" "/etc/systemd/system/$gateway"
systemctl daemon-reload
systemctl enable "$broker" "$gateway"
systemctl start "$broker" "$gateway"
# Type=notify establishes initialization, not a permanent availability promise.
# Verify its exact main-process invocation and protected executable twice before commit.
ready_instance() {
    local unit=$1 expected_uid=$2 pid invocation ready_text token again
    [[ $(systemctl show -p ActiveState --value "$unit") == active && $(systemctl show -p SubState --value "$unit") == running ]] || return 1
    pid=$(systemctl show -p MainPID --value "$unit")
    invocation=$(systemctl show -p InvocationID --value "$unit")
    ready_text=$(systemctl show -p StatusText --value "$unit")
    [[ $pid =~ ^[1-9][0-9]*$ && $invocation =~ ^[0-9a-f]{32}$
        && $ready_text == "Smart Explorer login ready version=$version generation=$generation sha256=$expected"
        && $(stat -c %u "/proc/$pid") == "$expected_uid"
        && $(readlink "/proc/$pid/exe") == "$image_path"
        && $(sha256sum "/proc/$pid/exe" | cut -d' ' -f1) == "$expected" ]] || return 1
    token="$pid:$invocation"
    again="$(systemctl show -p MainPID --value "$unit"):$(systemctl show -p InvocationID --value "$unit")"
    [[ $again == "$token" && $(systemctl show -p ActiveState --value "$unit") == active ]] || return 1
    printf '%s' "$token"
}
broker_instance=$(ready_instance "$broker" 0) || { echo 'Broker readiness/owned release instance could not be verified.' >&2; exit 1; }
gateway_instance=$(ready_instance "$gateway" "$uid") || { echo 'Gateway readiness/owned release instance could not be verified.' >&2; exit 1; }
[[ $(ready_instance "$broker" 0) == "$broker_instance" && $(ready_instance "$gateway" "$uid") == "$gateway_instance" ]] || { echo 'Login service instance changed before installation commit.' >&2; exit 1; }
rollback_active=0
echo "Optional login services ready for approved release $version; exact owned systemd invocations verified."
