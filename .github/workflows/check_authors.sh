#!/bin/bash

is_in_authors() {
  TARGET=$1
  if grep -Fq "$TARGET" AUTHORS; then
    return 1
  else
    return 0
  fi
}

# Only check the first 10 committers found in the PR
COMMITTERS=$(git log $1 --pretty=format:"%an;%ae" | sort | uniq | head -n 10)

for COMMITTER in "$COMMITTERS" ; do
  # split sentence in two parts seperated by a ;
  NAME=$(echo $COMMITTER | cut -d ";" -f 1)
  EMAIL=$(echo $COMMITTER | cut -d ";" -f 2)

  if is_in_authors "$EMAIL" == 0 && is_in_authors "$NAME" == 0; then
    echo "This author is not found in the AUTHORS file"
    exit 1
  fi
done

echo "All authors are found in the AUTHORS file."
exit 0
