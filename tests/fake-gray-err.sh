#!/bin/sh
printf '%s\n' '{"type":"progress","phase":"generating","protocol":1,"session_id":"fake-err","turn_id":"t-err"}'
printf '%s\n' '{"type":"error","code":"auth_failed","retryable":false,"message":"The provider rejected the credentials.","hint":"Check the API key","protocol":1,"session_id":"fake-err","turn_id":"t-err"}'
