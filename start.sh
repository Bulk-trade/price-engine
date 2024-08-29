#!/bin/sh
# start.sh
while true; do
    echo "Starting Bulk Price Engine..."
    /usr/local/bin/bulk_price_engine
    echo "Bulk Price Engine exited with code $?. Restarting in 5 seconds..."
    sleep 5
done