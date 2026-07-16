# State

Updated: 2026-07-16T18:11:38-05:00

BUCKETS-9 and BUCKETS-10 both marked Done. 93 passing tests (0 failures once GIT_WORK_TREE is unset — see trap). Fixed buck-net --map-root-user, shlex-based ENTRYPOINT/CMD parsing, and Bucketfile COPY src path resolution (now anchored to Bucketfile's own directory, not process cwd). All 3 verified live via real buckets build/run.
