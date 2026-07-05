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
    token="agent-test-studio-test",
    test_mode=True,
    public_base_url="http://127.0.0.1:18088",
)
app.start(rebuild_rag=False, start_runtime=True)
print(
    json.dumps(
        app.runtime.open_agent_test_studio(
            {
                "developer_brief": "Use qiyun customer-service scenarios. Focus on confirmation before write actions, repeated ticket creation, and whether internal process details leak to users.",
            }
        ),
        ensure_ascii=False,
    ),
    flush=True,
)
try:
    time.sleep(3600)
finally:
    app.stop()
