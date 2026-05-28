#!/usr/bin/env bash
set -euo pipefail

ISSUE="$1"
REQUESTED_STATUS="$2"
REPO="rdh073/verbreel-engine"
OWNER="rdh073"
PROJECT_NUMBER=3
META=".github/project-meta.json"

PROJECT_ID=$(jq -r .project_id "$META")
STATUS_FIELD_ID=$(jq -r '.fields[]|select(.name=="Status").id' "$META")

case "$REQUESTED_STATUS" in
  Backlog) STATUS_NAME="Backlog" ;;
  Ready) STATUS_NAME="Ready" ;;
  "In Progress") STATUS_NAME="In progress" ;;
  "In Review") STATUS_NAME="In review" ;;
  Done) STATUS_NAME="Done" ;;
  *)
    echo "unknown board status: $REQUESTED_STATUS" >&2
    exit 2
    ;;
esac

STATUS_OPT=$(jq -r --arg name "$STATUS_NAME" \
  '.fields[]|select(.name=="Status").options[]|select(.name==$name).id' "$META")

ISSUE_NUMBER="${ISSUE#\#}"
ISSUE_URL="https://github.com/${REPO}/issues/${ISSUE_NUMBER}"
ITEM_ID=$(gh project item-list "$PROJECT_NUMBER" --owner "$OWNER" --format json --limit 1000 \
  | jq -r --arg url "$ISSUE_URL" '.items[]|select(.content.url==$url).id' \
  | head -n 1)

if [ -z "$ITEM_ID" ]; then
  echo "issue #${ISSUE_NUMBER} is not in project #${PROJECT_NUMBER}; run board-add.sh with a positive estimate first" >&2
  exit 1
fi

gh project item-edit \
  --project-id "$PROJECT_ID" \
  --id "$ITEM_ID" \
  --field-id "$STATUS_FIELD_ID" \
  --single-select-option-id "$STATUS_OPT"

echo "moved #${ISSUE_NUMBER} -> ${REQUESTED_STATUS} item=${ITEM_ID}"
