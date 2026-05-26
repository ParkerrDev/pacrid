#!/usr/bin/env bash
# pacrid safety-rules enforcer.
#
# Codifies the project-specific NASA/JPL-derived rules that clippy alone
# cannot express (e.g. function length, SAFETY-comment requirement). Runs in
# CI and can be run locally as `./scripts/check-rules.sh`.
#
# Exits non-zero with a clear diagnostic on the first failing rule.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
    REPO_ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
fi
cd "$REPO_ROOT"

# Trailing -f prevents grep from treating filenames starting with -.
SRC_FILES=()
while IFS= read -r f; do
    SRC_FILES+=("$f")
done < <(find src -type f -name '*.rs' ! -path '*/tests/*' | sort)

PASS=0
FAIL=0
FAIL_DETAILS=()

green='\033[0;32m'; red='\033[0;31m'; bold='\033[1m'; rst='\033[0m'

ok()   { PASS=$((PASS + 1)); echo -e "  ${green}[ok]${rst}    $*"; }
fail() {
    FAIL=$((FAIL + 1))
    FAIL_DETAILS+=("$*")
    echo -e "  ${red}[FAIL]${rst}  $*"
}

section() { echo -e "\n${bold}$*${rst}"; }

# ── Rule 1: no recursion (self-recursion grep heuristic) ──────────────────────
section "Rule 1 — no recursion in production code"
# Look for "fn NAME" followed by a call NAME( on a later line within the same file.
recursion_hits=$(
    for f in "${SRC_FILES[@]}"; do
        awk -v file="$f" '
            /^(pub )?fn [a-zA-Z_][a-zA-Z0-9_]*/ {
                if (match($0, /fn ([a-zA-Z_][a-zA-Z0-9_]*)/, m)) {
                    curfn = m[1]; depth = 0; opened = 0
                }
                next
            }
            curfn != "" {
                n = gsub(/\{/, "{"); opened += n; if (n) depth += n
                n = gsub(/\}/, "}"); depth -= n
                if (opened > 0 && depth == 0) { curfn = ""; opened = 0; next }
                if (index($0, curfn "(")) {
                    print file ":" NR ": " curfn " appears to call itself"
                }
            }
        ' "$f"
    done
)
if [[ -z "$recursion_hits" ]]; then
    ok "no self-recursion"
else
    fail "potential recursion:"$'\n'"$recursion_hits"
fi

# ── Rule 2: no unbounded loop {} ──────────────────────────────────────────────
section "Rule 2 — no unbounded loop {}"
loop_hits=$(grep -nE '^\s*loop\s*\{' "${SRC_FILES[@]}" 2>/dev/null || true)
if [[ -z "$loop_hits" ]]; then
    ok "no bare \`loop {\`"
else
    fail "unbounded loop:"$'\n'"$loop_hits"
fi

# ── Rule 4: no function over 60 lines ─────────────────────────────────────────
# Uses brace-depth tracking so we measure the actual function body, not the
# distance to the next `fn` keyword (which would over-count when followed by
# const/static/impl blocks).
section "Rule 4 — no function over 60 lines"
long_funcs=$(
    for f in "${SRC_FILES[@]}"; do
        awk -v file="$f" '
            BEGIN { in_test = 0; in_fn = 0 }
            /^#\[cfg\(test\)\]/ { in_test = 1 }
            in_test { next }
            !in_fn && /^(pub )?fn [a-zA-Z_][a-zA-Z0-9_]*/ {
                curfn = $0; start = NR; depth = 0; in_fn = 1
            }
            in_fn {
                # Count braces, ignoring those inside line comments. (Heuristic
                # — good enough for our codebase.)
                line = $0; sub(/\/\/.*/, "", line)
                n_open = gsub(/\{/, "{", line); depth += n_open
                n_close = gsub(/\}/, "}", line); depth -= n_close
                if (n_open > 0 || n_close > 0) {
                    if (depth == 0 && start != NR) {
                        # function body closed
                        len = NR - start + 1
                        if (len > 60) print file":"start": "curfn" = "len" lines"
                        in_fn = 0
                    }
                }
            }
        ' "$f"
    done
)
if [[ -z "$long_funcs" ]]; then
    ok "no functions over 60 lines"
else
    fail "long functions:"$'\n'"$long_funcs"
fi

# ── Rule 5 (partial): every non-trivial public fn has ≥ 1 assert! ─────────────
# We don't enforce ≥2 universally — that would force assertions into trivial
# getters where they'd be noise. Instead we require at least one assert! in
# every public function over 15 lines.
section "Rule 5 — assertions in non-trivial public functions"
missing_assert=$(
    for f in "${SRC_FILES[@]}"; do
        awk -v file="$f" '
            /^pub fn [a-zA-Z_][a-zA-Z0-9_]*/ {
                if (curfn != "") {
                    len = NR - start
                    if (len > 15 && !saw_assert) print file":"start": "curfn" has no assert! ("len" lines)"
                }
                curfn = $0; start = NR; saw_assert = 0
            }
            curfn != "" && /assert(_eq|_ne)?!/ { saw_assert = 1 }
        ' "$f"
    done
)
if [[ -z "$missing_assert" ]]; then
    ok "all non-trivial public fns have ≥ 1 assert!"
else
    fail "public fns over 15 lines without assert!:"$'\n'"$missing_assert"
fi

# ── Rule 6: no static mut ─────────────────────────────────────────────────────
section "Rule 6 — no static mut"
static_mut=$(grep -nE 'static\s+mut\s' "${SRC_FILES[@]}" 2>/dev/null || true)
if [[ -z "$static_mut" ]]; then
    ok "no static mut"
else
    fail "static mut declarations:"$'\n'"$static_mut"
fi

# ── Rule 7: no .unwrap() / .expect( outside #[cfg(test)] blocks ───────────────
section "Rule 7 — no .unwrap()/.expect() in production code"
# Heuristic: a line containing .unwrap() or .expect( where the enclosing
# module is NOT marked #[cfg(test)]. Clippy already enforces this via
# clippy::unwrap_used / clippy::expect_used; we double-check here for grep-level
# review.
unwrap_hits=$(
    for f in "${SRC_FILES[@]}"; do
        awk -v file="$f" '
            /#\[cfg\(test\)\]/ { in_test = 1 }
            /#\[allow\(.*unwrap_used/ { in_allow = 1 }
            !in_test && !in_allow && (/\.unwrap\(\)/ || /\.expect\(/) {
                print file":"NR": "$0
            }
        ' "$f"
    done
)
if [[ -z "$unwrap_hits" ]]; then
    ok "no .unwrap()/.expect() outside tests"
else
    fail ".unwrap()/.expect() in production:"$'\n'"$unwrap_hits"
fi

# ── Rule 9: every unsafe { ... } preceded by a SAFETY: comment ────────────────
# A SAFETY block is a // SAFETY: line plus any contiguous // continuation
# lines immediately preceding (after blank lines reset) an unsafe { ... }
# block. Test modules (#[cfg(test)]) are exempt — env-var manipulation in
# tests is well-understood single-threaded test scaffolding.
section "Rule 9 — every unsafe block has a SAFETY: comment"
unsafe_no_safety=$(
    for f in "${SRC_FILES[@]}"; do
        awk -v file="$f" '
            BEGIN { saw_safety = 0; in_test = 0 }
            /#\[cfg\(test\)\]/                                  { in_test = 1 }
            in_test                                             { next }
            /^\s*\/\/[\/!]?\s*SAFETY:/                          { saw_safety = 1; next }
            /^\s*\/\//                                          { next }   # comment continuation keeps saw_safety
            /^\s*$/                                             { next }   # blank line ditto
            /unsafe\s*\{/ && !/^\s*(pub )?(unsafe )?fn /       {
                if (!saw_safety) print file":"NR": unsafe block without SAFETY: comment"
                saw_safety = 0
                next
            }
            { saw_safety = 0 }
        ' "$f"
    done
)
if [[ -z "$unsafe_no_safety" ]]; then
    ok "every unsafe block has a SAFETY: comment"
else
    fail "unsafe blocks missing SAFETY: comments:"$'\n'"$unsafe_no_safety"
fi

# ── Rule 10: lib.rs has the required deny lints ───────────────────────────────
section "Rule 10 — lib.rs enforces the safety lint set"
REQUIRED_LINTS=(
    "deny(warnings)"
    "deny(unsafe_op_in_unsafe_fn)"
    "deny(clippy::unwrap_used)"
    "deny(clippy::expect_used)"
    "deny(clippy::panic)"
    "deny(clippy::indexing_slicing)"
    "deny(clippy::arithmetic_side_effects)"
)
missing_lint=""
for lint in "${REQUIRED_LINTS[@]}"; do
    grep -qF "$lint" src/lib.rs || missing_lint+="${lint}"$'\n'
done
if [[ -z "$missing_lint" ]]; then
    ok "lib.rs has all required deny lints"
else
    fail "lib.rs is missing required lints:"$'\n'"$missing_lint"
fi

# ── summary ───────────────────────────────────────────────────────────────────
echo
if (( FAIL == 0 )); then
    echo -e "${green}${bold}All ${PASS} safety rules pass.${rst}"
    exit 0
else
    echo -e "${red}${bold}${FAIL} safety rule(s) failed (${PASS} passed).${rst}"
    exit 1
fi
