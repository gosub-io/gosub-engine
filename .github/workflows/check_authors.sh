#!/bin/bash

is_in_authors() {
  local target=$1
  if grep -Fq "$target" AUTHORS; then
    return 1
  else
    return 0
  fi
}

# Get all commits in this PR
COMMITTERS=$(git log -n $1 --pretty=format:"%an;%ae" | sort | uniq | grep -v noreply.github.com)

# shellcheck disable=SC2066
for committer in "$COMMITTERS" ; do
  echo "Validating author: $committer"

  # split sentence in two parts seperated by a ;
  name=$(echo $committer | cut -d ";" -f 1)
  email=$(echo $committer | cut -d ";" -f 2)

  if is_in_authors "$email" == 0 && is_in_authors "$name" == 0; then
    echo "Author $name <$email> was not found in the AUTHORS file"
    exit 1
  fi
done

echo "All authors are found in the AUTHORS file."
exit 0
