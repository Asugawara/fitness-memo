#!/bin/sh
set -eu

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit

npm install
npx playwright install chromium webkit
