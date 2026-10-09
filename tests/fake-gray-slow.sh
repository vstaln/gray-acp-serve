#!/bin/sh
# Like fake-gray.sh but sleeps before answering — session/cancel fodder.
session="${GRAY_FAKE_SESSION:-fake-slow}"
prompt=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) prompt="$2"; shift 2 ;;
    -p*) prompt="${1#-p}"; shift ;;
    --session) session="$2"; shift 2 ;;
    *) shift ;;
  esac
done
session="${session:-fake-slow}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"generating\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"t-slow\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"text\",\"segment\":0,\"delta\":\"thinking out loud\\n\",\"tail\":\"\",\"done\":false,\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"t-slow\"}"
sleep "${GRAY_FAKE_SLEEP:-30}"
printf '%s\n' "{\"type\":\"result\",\"text\":\"slow done\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"t-slow\"}"
