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
    token="combined-studios-test",
    test_mode=True,
    public_base_url="http://127.0.0.1:18088",
)
app.start(rebuild_rag=False, start_runtime=True)
workflow = app.runtime.open_workflow_studio(
    {"readonly": False, "open_browser": False}
)
agent_test = app.runtime.open_agent_test_studio(
    {
        "developer_brief": (
            "Use qiyun customer-service scenarios. Focus on confirmation before "
            "write actions, repeated ticket creation, and whether internal process "
            "details leak to users."
        )
    }
)
print(
    json.dumps(
        {
            "schema": "combined-studios-smoke/v1",
            "workflow_studio": workflow,
            "agent_test_studio": agent_test,
        },
        ensure_ascii=False,
    ),
    flush=True,
)
try:
    time.sleep(7200)
finally:
    app.stop()
