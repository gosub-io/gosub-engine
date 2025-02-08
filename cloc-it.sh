#!/bin/bash

mkdir -p weekly_loc
> loc_per_filetype.csv  # Create output CSV file

echo "Week,FileType,LOC" >> loc_per_filetype.csv

while read week; do

    git checkout main  --quiet

    # Get the last commit of the week$
    commit=$(git log --before="$week 23:59:59" --pretty=format:"%H" -n 1)
    echo "Commit: $commit  $week"

    if [ ! -z "$commit" ]; then
        # Check out the commit
        git checkout $commit --quiet

        # Run cloc and get LoC per file type
        files=$(git ls-files)
        cloc_output=$(cloc --quiet --json $files)
        echo "$cloc_output" > weekly_loc/$week.json

        # Parse JSON to get LoC per file type
        file_types=$(echo "$cloc_output" | jq -r 'to_entries[] | select(.key != "header" and .key != "SUM") | .key')
        for file_type in $file_types; do
            loc=$(echo "$cloc_output" | jq -r ".\"$file_type\".code")
            echo "$week,$file_type,$loc" >> loc_per_filetype.csv
        done
    fi
done < weeks.txt

# Return to the main branch
git checkout main --quiet

