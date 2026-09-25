#!/usr/bin/env bash
# Test servers of the Android task suite, sourced by android/test-android-task.sh (emulator-run).
# SFTP and FTP run as Docker containers on the CI runner; the emulator reaches the runner's
# loopback as 10.0.2.2 (docs/refs/android-ci.md §10). The update feed is a plain directory served
# over HTTP on the runner's loopback. WebDAV is deliberately absent: the app (like the desktop)
# connects to WebDAV over HTTPS only, which needs a publicly trusted certificate, so the suite
# checks only WebDAV validation and error kinds (explicit exception).
# Never run on the workstation (AGENTS.md); the remote runner calls it through the entry script.

SE_TASK_EMULATOR_HOST=10.0.2.2

SE_SFTP_IMAGE=atmoz/sftp:latest
SE_SFTP_CONTAINER=se-task-sftp
SE_SFTP_PORT=2222
SE_SFTP_USER=setester
SE_SFTP_PASS=se-task-sftp-pass
SE_SFTP_ROOT=/upload

# A full SSH server (exec allowed) for connections with the Remote-Agent, which deploys itself over
# SFTP plus exec channels; the user's home /config is writable.
SE_SSH_IMAGE=lscr.io/linuxserver/openssh-server:latest
SE_SSH_CONTAINER=se-task-ssh
SE_SSH_PORT=2223
SE_SSH_USER=seagent
SE_SSH_PASS=se-task-ssh-pass
SE_SSH_ROOT=/config

SE_FTP_IMAGE=delfer/alpine-ftp-server:latest
SE_FTP_CONTAINER=se-task-ftp
SE_FTP_PORT=21
SE_FTP_USER=seftp
SE_FTP_PASS=se-task-ftp-pass
SE_FTP_ROOT=/
SE_FTP_PASV_MIN=21000
SE_FTP_PASV_MAX=21010

SE_FEED_PORT=18080
SE_FEED_PID=""

# Reads the first bytes a TCP server sends after connect (banner check).
servers_banner() {
  local host=$1 port=$2
  timeout 5 bash -c "exec 3<>/dev/tcp/$host/$port && head -c 8 <&3" 2>/dev/null || true
}

servers_wait_banner() {
  local name=$1 port=$2 prefix=$3 deadline=$((SECONDS + 180)) banner=""
  while ((SECONDS < deadline)); do
    banner="$(servers_banner 127.0.0.1 "$port")"
    if [[ "$banner" == "$prefix"* ]]; then
      echo "test server $name ready on port $port (${banner%%$'\r'*})"
      return 0
    fi
    sleep 1
  done
  echo "test server $name did not answer with '$prefix' on port $port (last: '$banner')" >&2
  return 1
}

servers_up() {
  command -v docker >/dev/null 2>&1 || {
    echo "docker is required for the SFTP/FTP test servers" >&2
    return 1
  }
  docker rm -f "$SE_SFTP_CONTAINER" "$SE_SSH_CONTAINER" "$SE_FTP_CONTAINER" >/dev/null 2>&1 || true
  # atmoz/sftp: user:password:uid:gid:dir – the user is chrooted to its home, <dir> is writable.
  docker run -d --name "$SE_SFTP_CONTAINER" -p "$SE_SFTP_PORT:22" "$SE_SFTP_IMAGE" \
    "$SE_SFTP_USER:$SE_SFTP_PASS:::${SE_SFTP_ROOT#/}" >/dev/null
  docker run -d --name "$SE_SSH_CONTAINER" -p "$SE_SSH_PORT:2222" \
    -e PUID=1000 -e PGID=1000 -e TZ=Etc/UTC -e PASSWORD_ACCESS=true \
    -e USER_NAME="$SE_SSH_USER" -e USER_PASSWORD="$SE_SSH_PASS" \
    "$SE_SSH_IMAGE" >/dev/null
  # delfer/alpine-ftp-server: passive ports published 1:1 and advertised as the emulator's
  # address of the runner.
  docker run -d --name "$SE_FTP_CONTAINER" \
    -p "$SE_FTP_PORT:21" -p "$SE_FTP_PASV_MIN-$SE_FTP_PASV_MAX:$SE_FTP_PASV_MIN-$SE_FTP_PASV_MAX" \
    -e USERS="$SE_FTP_USER|$SE_FTP_PASS" \
    -e ADDRESS="$SE_TASK_EMULATOR_HOST" \
    -e MIN_PORT="$SE_FTP_PASV_MIN" -e MAX_PORT="$SE_FTP_PASV_MAX" \
    "$SE_FTP_IMAGE" >/dev/null
  servers_wait_banner SFTP "$SE_SFTP_PORT" "SSH-"
  servers_wait_banner SSH "$SE_SSH_PORT" "SSH-"
  # The image records the listener PID with `pgrep vsftpd | tail -n 1` right after start; a
  # probe connection before that makes it record the short-lived session child, and the
  # container exits when that session ends. So probe only once the PID file exists.
  servers_wait_ftp_pidfile
  servers_wait_banner FTP "$SE_FTP_PORT" "220"
  sleep 2
  if [[ "$(docker inspect -f '{{.State.Running}}' "$SE_FTP_CONTAINER" 2>/dev/null)" != "true" ]]; then
    echo "test server FTP container stopped after the banner check" >&2
    docker logs "$SE_FTP_CONTAINER" >&2 || true
    return 1
  fi
}

servers_wait_ftp_pidfile() {
  local deadline=$((SECONDS + 180))
  while ((SECONDS < deadline)); do
    if docker exec "$SE_FTP_CONTAINER" test -s /var/run/vsftpd/vsftpd.pid 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  echo "test server FTP did not write its PID file" >&2
  return 1
}

# Instrumentation arguments (-e name value) that tell the tests where the servers are.
servers_instrumentation_args() {
  printf '%s\n' \
    -e seServerHost "$SE_TASK_EMULATOR_HOST" \
    -e seSftpPort "$SE_SFTP_PORT" -e seSftpUser "$SE_SFTP_USER" -e seSftpPass "$SE_SFTP_PASS" -e seSftpRoot "$SE_SFTP_ROOT" \
    -e seSshPort "$SE_SSH_PORT" -e seSshUser "$SE_SSH_USER" -e seSshPass "$SE_SSH_PASS" -e seSshRoot "$SE_SSH_ROOT" \
    -e seFtpPort "$SE_FTP_PORT" -e seFtpUser "$SE_FTP_USER" -e seFtpPass "$SE_FTP_PASS" -e seFtpRoot "$SE_FTP_ROOT"
}

# Update feed: version.txt (next patch of native/Cargo.toml), the APK and its sha256sum sidecar,
# exactly the file names of release-native/update-feed (docs/RELEASING.md).
feed_prepare() {
  local dir=$1 apk=$2 version=$3
  mkdir -p "$dir"
  printf '%s\n' "$version" >"$dir/version.txt"
  cp -- "$apk" "$dir/smart-explorer-android.apk"
  (cd "$dir" && sha256sum smart-explorer-android.apk >smart-explorer-android.apk.sha256)
}

feed_up() {
  local dir=$1 log=$2 deadline=$((SECONDS + 60))
  python3 -m http.server "$SE_FEED_PORT" --bind 127.0.0.1 --directory "$dir" >"$log" 2>&1 &
  SE_FEED_PID=$!
  while ((SECONDS < deadline)); do
    if curl -fsS "http://127.0.0.1:$SE_FEED_PORT/version.txt" >/dev/null 2>&1; then
      echo "update feed served on port $SE_FEED_PORT"
      return 0
    fi
    sleep 0.5
  done
  echo "update feed did not start (log: $log)" >&2
  return 1
}

feed_url() {
  printf 'http://%s:%s\n' "$SE_TASK_EMULATOR_HOST" "$SE_FEED_PORT"
}

servers_down() {
  local logs=$1
  mkdir -p "$logs"
  local container
  for container in "$SE_SFTP_CONTAINER" "$SE_SSH_CONTAINER" "$SE_FTP_CONTAINER"; do
    docker logs "$container" >"$logs/$container.log" 2>&1 || true
  done
  docker exec "$SE_SFTP_CONTAINER" find /home >"$logs/sftp-tree.txt" 2>&1 || true
  docker exec "$SE_FTP_CONTAINER" find /ftp >"$logs/ftp-tree.txt" 2>&1 || true
  docker exec "$SE_SSH_CONTAINER" find /config -path /config/.cache -prune -o -print >"$logs/ssh-tree.txt" 2>&1 || true
  docker rm -f "$SE_SFTP_CONTAINER" "$SE_SSH_CONTAINER" "$SE_FTP_CONTAINER" >/dev/null 2>&1 || true
  if [[ -n "$SE_FEED_PID" ]]; then
    kill "$SE_FEED_PID" 2>/dev/null || true
    wait "$SE_FEED_PID" 2>/dev/null || true
    SE_FEED_PID=""
  fi
}
