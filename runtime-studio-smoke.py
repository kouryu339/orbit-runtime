import json
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent
EXAMPLE = ROOT / "examples" / "qiyunshanyoucha"
sys.path.insert(0, str(EXAMPLE))
sys.path.insert(0, str(ROOT / "sdk" / "python"))

from app import QiyunWechatApp  # noqa: E402


app = QiyunWechatApp(
    token="workflow-studio-test",
    test_mode=True,
    public_base_url="http://127.0.0.1:18088",
)
app.start(rebuild_rag=False, start_runtime=True)
print(
    json.dumps(
        app.runtime.open_workflow_studio({"readonly": False, "open_browser": False}),
        ensure_ascii=False,
    ),
    flush=True,
)
try:
    time.sleep(3600)
finally:
    app.stop()
