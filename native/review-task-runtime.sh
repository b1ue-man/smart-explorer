#!/usr/bin/env bash
# Linux runtime fixtures owned by the single RV1 suite. No build commands here.
set -Eeuo pipefail
[[ "${GITHUB_ACTIONS:-}" == true || "${SMART_EXPLORER_REMOTE_RUNNER:-}" == 1 ]] || exit 2
runtime="$(mktemp -d "$SMART_EXPLORER_TASK_LOG_ROOT/runtime.XXXXXX")"
chmod 700 "$runtime"
loops=()
mounts=()
sftp_pid=""
cleanup() {
  local status=$? cleanup_failed=0 mounted device
  trap - EXIT TERM INT
  set +e
  if mountpoint -q "$runtime/fuse"; then
    timeout 30 fusermount3 -u "$runtime/fuse" || cleanup_failed=1
  fi
  if [[ -n "$sftp_pid" ]]; then
    kill -TERM -- "-$sftp_pid" 2>/dev/null
    wait "$sftp_pid" 2>/dev/null
  fi
  for ((index=${#mounts[@]}-1; index>=0; index--)); do
    mounted=${mounts[$index]}
    if mountpoint -q "$mounted"; then timeout 30 sudo -n umount "$mounted" || cleanup_failed=1; fi
  done
  for device in "${loops[@]}"; do timeout 30 sudo -n losetup --detach "$device" || cleanup_failed=1; done
  if ((cleanup_failed)); then echo "Runtime cleanup failed; inspect $runtime" >&2; status=1; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
sudo -n true
for tool in losetup mkfs.fat mkfs.exfat mount mount.exfat-fuse umount sshfs fusermount3 socat udevadm; do
  command -v "$tool" >/dev/null || { echo "Missing runtime tool: $tool" >&2; exit 1; }
done
[[ -x /usr/lib/openssh/sftp-server ]] || { echo 'OpenSSH sftp-server missing' >&2; exit 1; }
for fs in fat exfat; do
  truncate -s 128M "$runtime/$fs.img"
  device="$(sudo -n losetup --find --show "$runtime/$fs.img")"
  loops+=("$device")
  if [[ $fs == fat ]]; then sudo -n mkfs.fat -F 32 "$device"; else sudo -n mkfs.exfat "$device"; fi
  mkdir "$runtime/$fs"
  mounts+=("$runtime/$fs")
  options="uid=$(id -u),gid=$(id -g),umask=077"
  if [[ $fs == fat ]]; then
    timeout 45 sudo -n mount -t vfat -o "$options" "$device" "$runtime/$fs"
  elif ! timeout 45 sudo -n mount -t exfat -o "$options" "$device" "$runtime/$fs"; then
    echo 'Kernel exFAT mount unavailable; using the real exFAT FUSE driver'
    timeout 45 sudo -n mount.exfat-fuse -o "$options" "$device" "$runtime/$fs"
  fi
  mountpoint -q "$runtime/$fs"
  sudo -n udevadm trigger --action=change "/sys/class/block/${device#/dev/}"
done
sudo -n udevadm settle --timeout=30
export SE_REVIEW_FAT_DIR="$runtime/fat" SE_REVIEW_EXFAT_DIR="$runtime/exfat"
mkdir "$runtime/remote" "$runtime/fuse"
port="$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    print(listener.getsockname()[1])
PY
)"
setsid socat "TCP4-LISTEN:$port,bind=127.0.0.1,reuseaddr,fork" EXEC:/usr/lib/openssh/sftp-server >"$runtime/sftp.log" 2>&1 &
sftp_pid=$!
python3 - "$port" <<'PY'
import socket,sys,time
deadline=time.monotonic()+30
while True:
    try:
        with socket.create_connection(('127.0.0.1',int(sys.argv[1])),timeout=1):
            break
    except OSError:
        if time.monotonic()>=deadline: raise
        time.sleep(.1)
PY
timeout 45 sshfs -o "directport=$port" "127.0.0.1:$runtime/remote" "$runtime/fuse"
mountpoint -q "$runtime/fuse"
export SE_REVIEW_NOREPLACE_DIR="$runtime/fuse"
printf 'fat=%s\nexfat=%s\nfuse=%s\nsftp_port=%s\n' "$SE_REVIEW_FAT_DIR" "$SE_REVIEW_EXFAT_DIR" "$SE_REVIEW_NOREPLACE_DIR" "$port" >"$runtime/values.txt"
"$@"
