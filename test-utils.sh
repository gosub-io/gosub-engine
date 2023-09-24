#!/bin/sh

# Simple test utils for bash scripts

reset="\e[0m"
expand="\e[K"

notice="\e[1;33;44m"
success="\e[1;33;42m"
fail="\e[1;33;41m"

function section() {
  SECTION=$1
  echo -e "\n"
  echo -e "${notice} $1 ${expand}${reset}"
  echo -e "\n"
}

function status() {
  RC=$?
  if [ "$RC" == "0" ] ; then
    echo -e "\n"
    echo -e "${success} ${expand}${reset}"
    echo -e "${success} SUCCESS: ${SECTION} ${expand}${reset}"
    echo -e "${success} ${expand}${reset}"
    echo -e "\n"
    echo -e "\n"
  else
    echo -e "\n"
    echo -e "${fail} ${expand}${reset}"
    echo -e "${fail} ERROR($RC): ${SECTION} ${expand}${reset}"
    echo -e "${fail} ${expand}${reset}"
    echo -e "\n"
    echo -e "\n"
  fi
}

trap "status" EXIT
