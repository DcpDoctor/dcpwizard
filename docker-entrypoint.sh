#!/bin/sh
set -e
# the rest api only queues jobs, the daemon runs them
if [ "$1" = "serve" ]; then
    dcpwizard daemon &
fi
exec dcpwizard "$@"
