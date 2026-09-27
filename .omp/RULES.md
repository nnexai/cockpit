# Cockpit delivery rules

- Say "fixed" only after running the affected scenario and seeing it work; otherwise say what is unverified.
- A feature that talks to an external service is done only after a live run against it. When the user has provided test credentials, use them, including creating test data there.
- Independent review only where a mistake is costly (data loss, concurrency, security); one round, high-severity findings only.
- Stay on the requested outcome; mention unrelated findings instead of fixing them. Stop once it works.
- Commit verified changes; leave unrelated work alone.
- Test Herdr changes only in disposable sessions (`python3 scripts/verify/ui_polish_runtime.py start|stop <root>`); never touch the user's session.
