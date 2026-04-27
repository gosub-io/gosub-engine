#!/bin/bash

is_in_authors() {
  local target=$1
  grep -Fq "$target" AUTHORS
}

# Get all commits in this PR
COMMITTERS=$(git log -n "$1" --pretty=format:"%an;%ae" | sort | uniq | grep -v noreply.github.com)

while IFS= read -r committer; do
  [ -z "$committer" ] && continue

  echo "Validating author: $committer"

  name=$(echo "$committer" | cut -d ";" -f 1)
  email=$(echo "$committer" | cut -d ";" -f 2)

  if ! is_in_authors "$email" && ! is_in_authors "$name"; then
    echo "Author $name <$email> was not found in the AUTHORS file"
    exit 1
  fi
done <<< "$COMMITTERS"

echo "All authors are found in the AUTHORS file."
exit 0
