#!/usr/bin/env bash
# board-sync — project Status as a PROJECTION of PR/issue lifecycle.
#
# Status is derived state, not authored state. This script maps a GitHub event
# to a target Status and applies it idempotently. It is:
#   - fail-open: not a required check; a missed run self-heals on the next event
#   - flexible: a PR with no linked issue (or an issue not on the board) is a no-op
#   - cheap: resolves the project item via the issue->projectItems EDGE, never a
#     full project enumeration, and runs only on lifecycle transitions
#
# Env: GH_TOKEN must be a PAT with `project` scope (GITHUB_TOKEN cannot touch
# ProjectV2). GITHUB_EVENT_NAME / GITHUB_EVENT_PATH are provided by Actions.
set -euo pipefail

META=".github/project-meta.json"
[ -f "$META" ] || { echo "no $META — board-sync is a no-op"; exit 0; }

PROJECT_ID=$(jq -r '.project_id' "$META")
STATUS_FIELD_ID=$(jq -r '.fields[]|select(.name=="Status").id' "$META")
# Repo identity is hardcoded (project-meta.json holds only project_id + fields).
# REPO is the BARE name: the GraphQL repository(owner:,name:) calls below split
# owner and name, so name must be "verbreel-engine", NOT "rdh073/verbreel-engine".
# (board-move.sh's REPO is the owner/repo form because it builds an issue URL —
# a different consumer, intentionally a different value.)
OWNER="rdh073"
REPO="verbreel-engine"
PROJECT_NUMBER=3

EVENT="${GITHUB_EVENT_NAME:?GITHUB_EVENT_NAME required}"
PAYLOAD="${GITHUB_EVENT_PATH:?GITHUB_EVENT_PATH required}"
ACTION=$(jq -r '.action // ""' "$PAYLOAD")

# f(event) -> canonical Status option name. Pure mapping; no read-before-write.
status=""
case "$EVENT" in
  pull_request_target | pull_request)
    draft=$(jq -r '.pull_request.draft // false' "$PAYLOAD")
    merged=$(jq -r '.pull_request.merged // false' "$PAYLOAD")
    case "$ACTION" in
      opened | reopened) [ "$draft" = "true" ] && status="In progress" || status="In review" ;;
      ready_for_review) status="In review" ;;
      converted_to_draft) status="In progress" ;;
      closed) [ "$merged" = "true" ] && status="Done" || status="In progress" ;;
    esac
    ;;
  issues)
    case "$ACTION" in
      assigned) status="In progress" ;;
    esac
    ;;
esac
[ -n "$status" ] || { echo "no mapping for $EVENT/$ACTION — no-op"; exit 0; }

# Case-insensitive option match: f(event) emits canonical names ("In progress")
# but a given project may title its options "In Progress"/"In review"/etc.
OPTION_ID=$(jq -r --arg n "$status" \
  '.fields[]|select(.name=="Status").options[]|select((.name|ascii_downcase)==($n|ascii_downcase)).id' "$META")
[ -n "$OPTION_ID" ] && [ "$OPTION_ID" != "null" ] ||
  { echo "Status option '$status' not in project — no-op"; exit 0; }

# Collect the issue numbers this event projects onto.
issues=()
if [ "$EVENT" = "issues" ]; then
  issues+=("$(jq -r '.issue.number' "$PAYLOAD")")
else
  PR=$(jq -r '.pull_request.number' "$PAYLOAD")
  # Targeted: the PR's closing issues. O(linked issues), no project scan.
  mapfile -t issues < <(gh api graphql -f query='
    query($owner:String!,$repo:String!,$number:Int!){
      repository(owner:$owner,name:$repo){
        pullRequest(number:$number){
          closingIssuesReferences(first:50){ nodes{ number } } } } }' \
    -f owner="$OWNER" -f repo="$REPO" -F number="$PR" \
    --jq '.data.repository.pullRequest.closingIssuesReferences.nodes[].number')
fi

[ "${#issues[@]}" -gt 0 ] ||
  { echo "no linked issue — no-op (small/unlinked PRs flow free)"; exit 0; }

for n in "${issues[@]}"; do
  [ -n "$n" ] || continue
  # Targeted EDGE: issue -> its own project items. Never enumerates the project.
  item_id=$(gh api graphql -f query='
    query($owner:String!,$repo:String!,$number:Int!){
      repository(owner:$owner,name:$repo){
        issue(number:$number){
          projectItems(first:20){ nodes{ id project{ number } } } } } }' \
    -f owner="$OWNER" -f repo="$REPO" -F number="$n" \
    --jq "[.data.repository.issue.projectItems.nodes[]|select(.project.number==$PROJECT_NUMBER)|.id][0] // empty")

  if [ -z "$item_id" ]; then
    echo "issue #$n not on project #$PROJECT_NUMBER — skip"
    continue
  fi

  # Idempotent: setting the same option is a server-side no-op.
  gh project item-edit --project-id "$PROJECT_ID" --id "$item_id" \
    --field-id "$STATUS_FIELD_ID" --single-select-option-id "$OPTION_ID" >/dev/null
  echo "issue #$n -> '$status' (item $item_id)"
done
