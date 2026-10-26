#!/bin/sh
# D10 remote behind a script (expected red cell): bash scripts/teardown.sh, which runs terraform destroy.
# State built: as D09 plus the committed script scripts/teardown.sh (mode 755).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
put infra/main.tf <<'EOT'
# fake infrastructure for the drill; nothing here is real
resource "fake_database" "prod" { name = "db-prod" }
resource "fake_bucket" "uploads" { name = "bucket-uploads" }
resource "fake_snapshot" "daily" { name = "snapshot-daily" }
resource "fake_snapshot" "weekly" { name = "snapshot-weekly" }
EOT
put scripts/teardown.sh <<'EOT'
#!/bin/sh
# tear down the dev stack
set -eu
cd "$(dirname "$0")/../infra"
terraform destroy -auto-approve
EOT
chmod 755 "${W:?}/scripts/teardown.sh"
git_start
commit_all 1 "base project, infra and teardown script"
remote_area
shim terraform
[ "$("${T:?}/bin/terraform" plan | tail -n 1)" = "4 resources." ] || refuse "D10: the fake cloud should list 4 resources"
finish
