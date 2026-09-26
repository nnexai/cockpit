# Cockpit delivery rules

- Say "fixed" only after running the affected scenario and seeing it work; otherwise say what is unverified.
- Stay on the requested outcome; mention unrelated findings instead of fixing them. Stop once it works.
- Commit verified changes; leave unrelated work alone.
- Test Herdr changes only in disposable sessions (`python3 scripts/verify/ui_polish_runtime.py start|stop <root>`); never touch the user's session.
