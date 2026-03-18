#!/bin/bash
# Watch for changes, rebuild, and restart schrottbar.
exec cargo watch -x 'build' -s 'pkill -x schrottbar; cargo run'
