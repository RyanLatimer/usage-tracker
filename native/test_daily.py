"""Regression tests for session-date aggregation used by the native dashboard."""
import json
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import usage_tracker as usage


class DailyUsageTests(unittest.TestCase):
    def test_dates_are_utc_and_invalid_dates_stay_unknown(self):
        self.assertEqual(usage.session_day('2026-11-02T23:30:00-05:00'), '2026-11-03')
        self.assertEqual(usage.session_day('2026-11-02T00:30:00+02:00'), '2026-11-01')
        self.assertEqual(usage.session_day(1793577600000), '2026-11-02')
        self.assertIsNone(usage.session_day(None))
        self.assertIsNone(usage.session_day('invalid'))

    def test_codex_days_reconcile_with_all_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            root = home / '.codex/sessions'
            root.mkdir(parents=True)
            for i, date in enumerate(['2026-11-01', '2026-11-02']):
                records = [
                    {'type': 'session_meta', 'timestamp': date+'T12:00:00Z', 'payload': {'cwd': '/project'}},
                    {'type': 'turn_context', 'payload': {'model': 'gpt-6-sol'}},
                    {'type': 'event_msg', 'payload': {'type': 'token_count', 'info': {'total_token_usage': {
                        'input_tokens': 1000*(i+1), 'cached_input_tokens': 200, 'output_tokens': 100}}}},
                ]
                (root / f'{i}.jsonl').write_text('\n'.join(map(json.dumps, records)))
            with patch.object(usage, 'HOME', home):
                result = usage.collect_codex(usage.DEFAULT_PRICING, None)
            self.assertEqual(len(result.daily), 2)
            combined = usage.ModelStats()
            for bucket in result.daily.values():
                for st in bucket.values():
                    combined.add(st)
            self.assertEqual(combined.sessions, result.total.sessions)
            self.assertEqual(combined.input_tokens, 2600)
            self.assertEqual(combined.cache_read, 400)
            self.assertAlmostEqual(combined.cost_usd, result.total.cost_usd)

    def test_opencode_missing_dates_preserves_all_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            root = home / '.local/share/opencode'
            root.mkdir(parents=True)
            con = sqlite3.connect(root / 'opencode.db')
            con.execute('CREATE TABLE session (id, model, cost, tokens_input, tokens_output, tokens_cache_read, tokens_cache_write, tokens_reasoning, directory)')
            con.execute("INSERT INTO session VALUES ('s','gpt-6-sol',0,1000,100,0,0,0,'/project')")
            con.execute('CREATE TABLE message (id, session_id, data)')
            con.commit()
            con.close()
            with patch.object(usage, 'HOME', home):
                result = usage.collect_opencode(usage.DEFAULT_PRICING, None)
            self.assertEqual(result.total.sessions, 1)
            self.assertGreater(result.total.cost_usd, 0)
            self.assertEqual(result.daily, {})


if __name__ == '__main__':
    unittest.main()
