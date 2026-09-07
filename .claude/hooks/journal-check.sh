#!/usr/bin/env bash
# Stop hook: refuse to finish while commits exist that no docs/JOURNAL.md entry
# covers (CLAUDE.md, "End of every session").
#
# A Stop hook fires at the end of every turn, not just the session, so an
# unconditional block would interrupt all day. Two guards keep the dose to one:
#   - `stop_hook_active` is true on the continuation this hook itself caused,
#     so it never loops.
#   - a per-session marker means it blocks at most once per session; after that
#     the obligation is CLAUDE.md's, not the hook's.
# Exit 2 blocks and sends stderr back to the model. Any other failure exits 0 —
# a broken hook must never wedge a session.
set -uo pipefail

input=$(cat 2>/dev/null || true)
field() { printf '%s' "$input" | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\{0,1\}\([^\",}]*\)\"\{0,1\}.*/\1/p" | head -1; }

[ "$(field stop_hook_active)" = "true" ] && exit 0

cd "${CLAUDE_PROJECT_DIR:-.}" 2>/dev/null || exit 0
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

session=$(field session_id)
marker="${TMPDIR:-/tmp}/gridwatch-journal-${session:-unknown}"
[ -n "$session" ] && [ -e "$marker" ] && exit 0

journal="docs/JOURNAL.md"
last=$(git log -1 --format=%H -- "$journal" 2>/dev/null)
if [ -z "$last" ]; then
  # Never committed. Only nag once there is history to write about.
  git log -1 --format=%H >/dev/null 2>&1 || exit 0
  pending=$(git log --oneline -20 2>/dev/null | wc -l | tr -d ' ')
else
  pending=$(git log --oneline "${last}..HEAD" 2>/dev/null | wc -l | tr -d ' ')
fi
[ "${pending:-0}" -gt 0 ] 2>/dev/null || exit 0

[ -n "$session" ] && : > "$marker" 2>/dev/null
subjects=$(git log --format='  %h %s' "${last:+${last}..}HEAD" 2>/dev/null | head -12)

cat >&2 <<MSG
$pending commit(s) are not covered by a docs/JOURNAL.md entry:

$subjects

CLAUDE.md ("End of every session") asks for one entry per session, newest
first, written while the work is still in context and committed with the
session's last commit. Lead with what changed for Matt and why it matters,
not an inventory of edits. Record what the reviews caught, what turned out
to be wrong, the near misses, and what is owed to him.

Write or extend the newest entry now, then commit it. If the work genuinely
does not warrant one, say so in your reply and finish — this fires once per
session.
MSG
exit 2
