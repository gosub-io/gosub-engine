#!/usr/bin/env bash

# Test utilities for Makefile targets

reset="\e[0m"
expand="\e[K"

notice="\e[1;33;44m"
success="\e[1;33;42m"
fail="\e[1;33;41m"

# run_section "Title" <command> [args...]
#
# Streams command output live to the terminal. Once the command finishes the
# output is collapsed and replaced by a single coloured pass/fail line.
# When stdout is not a TTY (e.g. CI pipe) the collapse is skipped and output
# streams normally, followed by the summary line.
function run_section() {
  local title="$1"
  shift

  local tmpfile rc
  local is_tty=false
  [ -t 1 ] && is_tty=true

  tmpfile=$(mktemp)

  echo -e "${notice} ▶  ${title} ${expand}${reset}"

  if $is_tty; then
    printf '\e7'          # DECSC – save cursor position
  fi

  # Run command; tee captures output for the failure dump while still
  # streaming it live to the terminal.
  "$@" 2>&1 | tee "$tmpfile"
  rc=${PIPESTATUS[0]}

  if $is_tty; then
    printf '\e8\e[J'      # DECRC + erase to end of screen → collapse output
  fi

  if [ "$rc" = "0" ]; then
    echo -e "${success} ✓  PASS  ${title} ${expand}${reset}"
  else
    echo -e "${fail} ✗  FAIL  ${title} ${expand}${reset}"
    # On failure in TTY mode the output was collapsed, so dump it again.
    if $is_tty; then
      echo -e "\n--- output ---"
      cat "$tmpfile"
      echo -e "--- end ---\n"
    fi
  fi

  rm -f "$tmpfile"
  return "$rc"
}

# Legacy helper kept for any scripts that still call section/status directly.
function section() {
  SECTION=$1
  echo -e "\n"
  echo -e "${notice} $1 ${expand}${reset}"
  echo -e "\n"
}
