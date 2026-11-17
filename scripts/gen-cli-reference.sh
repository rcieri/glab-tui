#!/usr/bin/env bash
# Regenerates docs/cli-reference.md from the --help of the glab and gh
# versions pinned below. The e2e harness checks every call the CLI mocks
# record against that file (tests/e2e/cli_contract.rs), so bump the pins and
# rerun this after upgrading either CLI.
# The backticks in the printf formats are Markdown, not command substitution.
# shellcheck disable=SC2016
set -euo pipefail

GLAB_VERSION=1.122.0
GH_VERSION=2.102.0

cd "$(dirname "$0")/.."
out=docs/cli-reference.md

export NO_COLOR=1 CLICOLOR=0 GH_NO_UPDATE_NOTIFIER=1 GH_PROMPT_DISABLED=1 \
    GLAB_CHECK_UPDATE=false GLAB_SEND_TELEMETRY=false

require_version() {
    local cli=$1 pinned=$2 installed
    installed=$("$cli" --version 2>&1 | head -1)
    # A whole word, so 1.122.0-rc1 does not pass for 1.122.0.
    if [[ " $installed " != *" $pinned "* ]]; then
        echo "$cli $pinned is pinned, but the installed one says: $installed" >&2
        exit 1
    fi
}

# Reads one command's help on stdin and prints tab-separated records:
#   U <usage>              the first usage line
#   A <alias>              another spelling of this command (gh ALIASES)
#   C <name> <summary>     a subcommand
#   F <spelling> <summary> a flag from FLAGS or INHERITED FLAGS
#   X <flag>               a flag no FLAGS section lists but an example passes
#                          to exactly this command, not to a subcommand of it
#                          (glab's -R on runner update). Quoted words, such as
#                          the body of an alias, are not read.
# gh's EXTENSION and ALIAS COMMANDS are left out: they are local setup.
parse_help() {
    awk -v path="$1" '
    function emit_flag() {
        if (spec != "") printf "F\t%s\t%s\n", spec, summary
        spec = ""
    }
    {
        sub(/[ \t]+$/, "")
        line = $0
        sub(/^[ \t]+/, "", line)
        if (line ~ /^[A-Z][A-Z ]*[A-Z]$/) { emit_flag(); section = line; next }
        if (line == "") next
        if (section == "USAGE") {
            if (!usage_seen) { printf "U\t%s\n", line; usage_seen = 1 }
            next
        }
        if (section == "ALIASES") {
            # gh puts several aliases on one line: "gh codespace ls, gh cs ls".
            n = split(line, entries, /, */)
            for (i = 1; i <= n; i++) if (entries[i] != "") printf "A\t%s\n", entries[i]
            next
        }
        if (section == "EXAMPLES") {
            if (index(line, path " ") == 1) examples[++example_count] = substr(line, length(path) + 2)
            next
        }
        if (section ~ /COMMANDS$/ && section != "EXTENSION COMMANDS" && section != "ALIAS COMMANDS") {
            n = split(line, parts, /  +/)
            name = parts[1]
            sub(/[ \t].*/, "", name)
            sub(/:$/, "", name)
            # gh writes "name: summary", padded to align but with one space
            # after the longest name. glab puts the usage between the name
            # and the summary, each column two or more spaces apart.
            if (line ~ /^[a-z0-9-]+:/) { text = line; sub(/^[^:]+:[ \t]*/, "", text) }
            else text = n > 1 ? parts[n] : ""
            if (name ~ /^[a-z0-9][a-z0-9-]*$/) {
                printf "C\t%s\t%s\n", name, text
                child[name] = 1
            }
            next
        }
        if (section ~ /FLAGS$/) {
            if (line ~ /^-/) {
                emit_flag()
                spec = line
                summary = ""
                if (match(line, /  +/)) {
                    spec = substr(line, 1, RSTART - 1)
                    summary = substr(line, RSTART + RLENGTH)
                }
                split(spec, words, /[ ,]+/)
                for (i in words) if (words[i] ~ /^-/) listed_flag[words[i]] = 1
            } else if (spec != "") {
                summary = summary " " line
            }
        }
    }
    END {
        emit_flag()
        for (e = 1; e <= example_count; e++) {
            n = split(examples[e], words, /[ \t]+/)
            if (words[1] in child) continue
            for (i = 1; i <= n; i++) {
                w = words[i]
                # Quoted text and what follows "--" belong to another program.
                if (w == "--" || w ~ /^["\047]/) break
                sub(/=.*/, "", w)
                if (w ~ /^(-[A-Za-z]|--[A-Za-z][A-Za-z0-9-]*)$/) example_flag[w] = 1
            }
        }
        for (f in example_flag) if (!(f in listed_flag)) printf "X\t%s\n", f
    }'
}

escape_cell() {
    printf '%s' "${1//|/\\|}"
}

# Writes the section of one command, then of each of its subcommands.
document() {
    local path=$1 depth=$2 tag first rest usage="" help hashes records
    local -a aliases=() children=() child_lines=() flag_lines=() example_flags=()
    local -a words
    read -ra words <<<"$path"
    hashes=$(printf '#%.0s' $(seq 1 $((depth < 5 ? depth + 1 : 6))))
    printf '%s `%s`\n\n' "$hashes" "$path"
    # Run from an empty directory: some help (glab dependency-firewall)
    # depends on the git remote of the current directory.
    if ! help=$(cd "$empty_dir" && "${words[@]}" --help 2>&1); then
        local reason
        # The first line of the error, up to its first ": ": the rest names
        # local paths ("… up to mount point /").
        reason=$(awk 'NF && $0 !~ /^ *ERROR *$/ { sub(/^ +/, ""); sub(/: .*/, ""); sub(/ +$/, ""); print; exit }' <<<"$help")
        printf '**Help unavailable:** `%s`\n\n' "$reason"
        return
    fi
    # Assigned first, so a failing parse_help stops the script.
    records=$(parse_help "$path" <<<"$help")
    while IFS=$'\t' read -r tag first rest; do
        case $tag in
            U) usage=$first ;;
            A) aliases+=("$first") ;;
            C)
                children+=("$first")
                child_lines+=("- \`$first\` — $rest")
                ;;
            F) flag_lines+=("| \`$first\` | $(escape_cell "$rest") |") ;;
            X) example_flags+=("$first") ;;
        esac
    done <<<"$records"
    if ((${#example_flags[@]})); then
        local flag
        while read -r flag; do
            flag_lines+=("| \`$flag\` | Not listed under FLAGS; the help's examples pass it. |")
        done < <(printf '%s\n' "${example_flags[@]}" | LC_ALL=C sort)
    fi

    # A word a COMMANDS section wraps onto its own line would be read as a
    # subcommand, and cobra answers its --help with the parent's help.
    if [[ -n $usage && $usage != "$path" && $usage != "$path "* ]]; then
        echo "\`$path --help\` shows the usage of another command: $usage" >&2
        exit 1
    fi
    [[ -n $usage ]] && printf '**Usage:** `%s`\n\n' "$usage"
    if ((${#aliases[@]})); then
        local joined
        joined=$(printf '`%s`, ' "${aliases[@]}")
        printf '**Aliases:** %s\n\n' "${joined%, }"
    fi
    if ((${#child_lines[@]})); then
        printf '**Subcommands:**\n'
        printf '%s\n' "${child_lines[@]}"
        printf '\n'
    fi
    if ((${#flag_lines[@]})); then
        printf '**Flags / Options:**\n\n| Flag | Description |\n|---|---|\n'
        printf '%s\n' "${flag_lines[@]}"
        printf '\n'
    fi
    # Guarded: bash 3.2 (macOS) treats an empty array as unset under set -u.
    ((${#children[@]})) || return 0
    local child
    for child in "${children[@]}"; do
        document "$path $child" $((depth + 1))
    done
}

require_version glab "$GLAB_VERSION"
require_version gh "$GH_VERSION"

tmp=$(mktemp)
empty_dir=$(mktemp -d)
trap 'rm -rf "$tmp" "$empty_dir"' EXIT
{
    printf '# `glab` / `gh` CLI Command & Flag Reference\n\n'
    printf 'Every `glab` and `gh` subcommand and option of `glab v%s` / `gh v%s`, generated by `scripts/gen-cli-reference.sh` from their `--help`. The e2e tests check every call the CLI mocks record against this file. Which of these glab-tui actually calls, and why, is documented in the "CLI Command Mapping" section of [AGENTS.md](../AGENTS.md).\n\n' \
        "$GLAB_VERSION" "$GH_VERSION"
    printf '## GitLab CLI (`glab`) Reference\n\n'
    document glab 1
    printf '## GitHub CLI (`gh`) Reference\n\n'
    document gh 1
} >"$tmp"
# One trailing newline, as the file had before.
sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' "$tmp" >"$out"
echo "$out: glab $GLAB_VERSION, gh $GH_VERSION, $(grep -cE '^#{2,6} `' "$out") commands"
