#!/usr/bin/env bash
set -euo pipefail
ISSUE_NUMBER="$1"
ESTIMATE_POINTS="${2:?usage: board-add.sh ISSUE_NUMBER POSITIVE_ESTIMATE_POINTS}"
REPO="rdh073/verbreel-engine"
OWNER="rdh073"
PROJECT_NUMBER=3
META=".github/project-meta.json"
PROJECT_ID=$(jq -r .project_id "$META")
STATUS_FIELD_ID=$(jq -r '.fields[]|select(.name=="Status").id' "$META")
ESTIMATE_FIELD_ID=$(jq -r '.fields[]|select(.name=="Estimate").id' "$META")
BACKLOG_OPT=$(jq -r '.fields[]|select(.name=="Status").options[]|select(.name=="Backlog").id' "$META")

case "$ESTIMATE_POINTS" in
  ''|*[!0-9]*)
    echo "estimate must be a positive integer: $ESTIMATE_POINTS" >&2
    exit 2
    ;;
esac

if [ "$ESTIMATE_POINTS" -le 0 ]; then
  echo "estimate must be greater than zero for Backlog items" >&2
  exit 2
fi

ITEM_ID=$(gh project item-add "$PROJECT_NUMBER" --owner "$OWNER" \
  --url "https://github.com/${REPO}/issues/${ISSUE_NUMBER}" \
  --format json | jq -r .id)

gh project item-edit \
  --project-id "$PROJECT_ID" \
  --id "$ITEM_ID" \
  --field-id "$STATUS_FIELD_ID" \
  --single-select-option-id "$BACKLOG_OPT"

gh project item-edit \
  --project-id "$PROJECT_ID" \
  --id "$ITEM_ID" \
  --field-id "$ESTIMATE_FIELD_ID" \
  --number "$ESTIMATE_POINTS"

echo "added #${ISSUE_NUMBER} -> Backlog (est ${ESTIMATE_POINTS}p) item=${ITEM_ID}"
