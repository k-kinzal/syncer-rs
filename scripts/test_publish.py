import contextlib
import io
import subprocess
import unittest
from email.utils import parsedate_to_datetime
from unittest.mock import patch

import publish


RATE_LIMIT = (
    "the remote server responded with an error (status 429 Too Many Requests): "
    "You have published too many new crates in a short period of time. "
    "Please try again after Sun, 27 Sep 2026 10:40:52 GMT "
    "and see https://crates.io/docs/rate-limits for more details."
)


class PublishTests(unittest.TestCase):
    def test_waits_until_server_time_then_retries(self):
        server_time = parsedate_to_datetime("Sun, 27 Sep 2026 10:40:52 GMT").timestamp()
        now = [server_time - 125]

        def sleep(seconds):
            now[0] += seconds

        results = [
            subprocess.CompletedProcess([], 101, "", RATE_LIMIT),
            subprocess.CompletedProcess([], 0, "Published", ""),
        ]
        with (
            patch.object(publish, "is_published", side_effect=[False, False, True]),
            patch.object(publish.subprocess, "run", side_effect=results) as run,
            patch.object(publish.time, "time", side_effect=lambda: now[0]),
            patch.object(publish.time, "sleep", side_effect=sleep) as wait,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            publish.publish("syncer-example", "0.1.0")
        self.assertEqual(run.call_count, 2)
        self.assertEqual([call.args[0] for call in wait.call_args_list], [60, 60, 7])

    def test_other_errors_are_not_retried(self):
        result = subprocess.CompletedProcess([], 101, "", "A verified email address is required")
        with (
            patch.object(publish, "is_published", return_value=False),
            patch.object(publish.subprocess, "run", return_value=result) as run,
            patch.object(publish.time, "sleep") as wait,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            with self.assertRaises(subprocess.CalledProcessError):
                publish.publish("syncer-example", "0.1.0")
        run.assert_called_once()
        wait.assert_not_called()

    def test_existing_version_is_not_uploaded(self):
        with (
            patch.object(publish, "is_published", return_value=True),
            patch.object(publish.subprocess, "run") as run,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            publish.publish("syncer-example", "0.1.0")
        run.assert_not_called()

    def test_unrecognized_rate_limit_fails_closed(self):
        self.assertIsNone(publish.retry_at("status 429 Too Many Requests"))
        self.assertIsNone(publish.retry_at(RATE_LIMIT.replace("10:40:52", "invalid")))
        self.assertIsNone(publish.retry_at(RATE_LIMIT.replace("status 429", "status 500")))


if __name__ == "__main__":
    unittest.main()
