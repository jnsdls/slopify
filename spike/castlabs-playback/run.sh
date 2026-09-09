#!/bin/zsh
# Run the spike without re-signing (node_modules already carries a valid VMP signature).
cd "$(dirname "$0")"
rm -f spike.log
env -u ELECTRON_RUN_AS_NODE pnpm exec electron . 2>&1 | grep -vE 'CoreText|font'
