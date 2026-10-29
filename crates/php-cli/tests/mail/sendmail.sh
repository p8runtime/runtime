#!/bin/sh
printf "argv:%s\n" "$*" >> "$MAILBOX"
cat >> "$MAILBOX"
printf "\n--\n" >> "$MAILBOX"
