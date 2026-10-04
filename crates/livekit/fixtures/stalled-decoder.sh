#!/bin/sh
# A stand-in for a decoder that has not reached its input loop: it ignores the
# arguments it is handed, never reads its stdin, and never writes its stdout.
#
# Committed rather than written by the test that uses it. A test process that
# creates an executable and immediately execs it loses a race it cannot see: a
# sibling thread spawning any child forks a copy of the still-open write
# descriptor, and Linux refuses to exec a file that any process holds open for
# writing. That surfaced as ETXTBSY once in roughly 120 runs, reported as "no
# H.264 decoder binary is available" for a binary that was right there.
exec sleep 300
