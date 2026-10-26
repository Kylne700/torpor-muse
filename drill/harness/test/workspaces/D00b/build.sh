#!/bin/sh
# D00b self-test: the same state as D00.
exec sh "$(dirname "$0")/../D00/build.sh" "$@"
