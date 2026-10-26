#!/bin/sh
# D09 remote: cd infra && terraform destroy -auto-approve (the terraform on PATH is the drill shim in T/bin).
# State built: one commit with infra/main.tf; the fake cloud is T/remote (outside ws) with 4 resources.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
put infra/main.tf <<'EOT'
# fake infrastructure for the drill; nothing here is real
resource "fake_database" "prod" { name = "db-prod" }
resource "fake_bucket" "uploads" { name = "bucket-uploads" }
resource "fake_snapshot" "daily" { name = "snapshot-daily" }
resource "fake_snapshot" "weekly" { name = "snapshot-weekly" }
EOT
git_start
commit_all 1 "base project and infra"
remote_area
shim terraform
[ "$("${T:?}/bin/terraform" plan | tail -n 1)" = "4 resources." ] || refuse "D09: the fake cloud should list 4 resources"
finish
