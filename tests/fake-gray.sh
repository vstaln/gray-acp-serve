#!/bin/sh
# Fake `gray -p --json` for offline gray-acp testing: emits the same NDJSON
# wire (protocol 1) without any provider call. Honors --session (echoes it
# back as session_id), -p (echoes the prompt), and ignores the rest.
prompt=""
session=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) prompt="$2"; shift 2 ;;
    -p*) prompt="${1#-p}"; shift ;;
    --session) session="$2"; shift 2 ;;
    *) shift ;;
  esac
done
session="${session:-fake-gray-session}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"generating\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"thinking\",\"detail\":\"fake reasoning\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"text\",\"segment\":0,\"delta\":\"fake echo: \",\"tail\":\"$prompt\",\"done\":false,\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"tool_started\",\"tool\":\"bash\",\"label\":\"Bash\",\"call_id\":\"c1\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"tool_ran\",\"tool\":\"bash\",\"label\":\"Bash\",\"call_id\":\"c1\",\"detail\":\"echo hi\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"tool_finished\",\"tool\":\"bash\",\"label\":\"Bash\",\"call_id\":\"c1\",\"output\":\"hi\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"text\",\"segment\":0,\"delta\":\"$prompt\",\"tail\":\"\",\"done\":true,\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"progress\",\"phase\":\"persisting\",\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
printf '%s\n' "{\"type\":\"result\",\"text\":\"fake echo: $prompt\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15},\"protocol\":1,\"session_id\":\"$session\",\"turn_id\":\"fake-turn\"}"
