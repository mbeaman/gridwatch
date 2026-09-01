#!/usr/bin/env bash
# The PERFORMANCE.md measurement protocol (unprivileged; perf/strace are
# sudo-only on torch). Run inside the terminal being measured.
#   scripts/perf/measure.sh <label> [seconds]
set -euo pipefail
LABEL="${1:?usage: measure.sh <label> [seconds]}"
SECS="${2:-60}"
PID="$(pidof gridwatch | awk '{print $1}')" || { echo "gridwatch not running"; exit 1; }
PTYXIS="$(pgrep -x ptyxis | head -1 || true)"
DATE="$(date +%F)"
SIZE="$(stty size 2>/dev/null | awk '{print $2"x"$1}' || echo '?')"

vol() { awk '/^voluntary_ctxt_switches/{s+=$2} END{print s}' /proc/"$1"/task/*/status 2>/dev/null || echo 0; }
wch() { awk '/^wchar/{print $2}' /proc/"$1"/io 2>/dev/null || echo 0; }

V0="$(vol "$PID")"; W0="$(wch "$PID")"
echo "sampling $SECS s: gridwatch pid $PID (terminal $SIZE)…"
pidstat -u -r -t -p "$PID" 1 "$SECS" > /tmp/gw-pidstat.txt &
PIDSTAT=$!
[ -n "$PTYXIS" ] && pidstat -u -p "$PTYXIS" 1 "$SECS" > /tmp/gw-ptyxis.txt &
command -v nvidia-smi >/dev/null && nvidia-smi pmon -s u -d 1 -c "$SECS" > /tmp/gw-pmon.txt 2>/dev/null &
wait "$PIDSTAT" 2>/dev/null || true
wait 2>/dev/null || true
V1="$(vol "$PID")"; W1="$(wch "$PID")"

CPU="$(awk '/Average:/ && $NF=="gridwatch" {print $8}' /tmp/gw-pidstat.txt | head -1)"
PTX="$([ -n "$PTYXIS" ] && awk '/Average:/ && !/Command/ {print $8}' /tmp/gw-ptyxis.txt | head -1 || echo '-')"
SM="-"
if [ -s /tmp/gw-pmon.txt ] && [ -n "$PTYXIS" ]; then
  SM="$(awk -v p="$PTYXIS" '$2==p {if ($4=="-") n++; else s+=$4; c++} END{if(c>0) printf "%.1f", s/c; else print "-"}' /tmp/gw-pmon.txt)"
fi
WAKES="$(( (V1 - V0) / SECS ))"
BYTES="$(( (W1 - W0) / SECS ))"
RSS="$(awk '/VmRSS/{print $2" "$3}' /proc/"$PID"/status)"

ROW="| $DATE | 1 | quiet | $LABEL | ? | ${CPU:-?}% | $WAKES | $((BYTES / 1024)) KB/s | ? | ${PTX}% | $SM | ? | ? | ? | $RSS |"
echo "$ROW"
echo "$ROW" >> "$(dirname "$0")/../../docs/PERFORMANCE.md"
echo "appended to docs/PERFORMANCE.md — fill frames/NVML/scan columns from the F12 HUD dump"
