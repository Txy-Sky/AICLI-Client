import json
import threading
import urllib.error
import urllib.request

import pytest

from mock_upstream.server import Handler, State
from http.server import ThreadingHTTPServer


@pytest.fixture()
def server():
    import mock_upstream.server as module
    old = module.STATE
    module.STATE = State()
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_port}"
    httpd.shutdown()
    thread.join(timeout=2)
    module.STATE = old


def request(base, path, payload):
    data = json.dumps(payload).encode()
    req = urllib.request.Request(base + path, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=3) as response:
        return response.status, response.headers.get_content_type(), response.read().decode()


@pytest.mark.parametrize("path,payload", [
    ("/v1/chat/completions", {"model": "mock-model", "messages": [], "stream": False}),
    ("/v1/responses", {"model": "mock-model", "input": "hi", "stream": False}),
    ("/v1/messages", {"model": "mock-model", "messages": [], "stream": False}),
    ("/v1beta/models/mock-model:generateContent", {"contents": []}),
])
def test_non_stream_protocols(server, path, payload):
    status, content_type, body = request(server, path, payload)
    assert status == 200
    assert content_type == "application/json"
    assert body


@pytest.mark.parametrize("path,payload", [
    ("/v1/chat/completions", {"model": "mock-model", "messages": [], "stream": True}),
    ("/v1/responses", {"model": "mock-model", "input": "hi", "stream": True}),
    ("/v1/messages", {"model": "mock-model", "messages": [], "stream": True}),
    ("/v1beta/models/mock-model:streamGenerateContent", {"contents": []}),
])
def test_stream_protocols(server, path, payload):
    status, content_type, body = request(server, path, payload)
    assert status == 200
    assert content_type == "text/event-stream"
    assert "data:" in body


@pytest.mark.parametrize("control", [
    {"delay_first_byte": 0.01},
    {"idle_gap": 0.01},
    {"fail_status": 429},
    {"fail_after_n_chunks": 1},
    {"stream_error_event": True},
])
def test_control_injections(server, control):
    request(server, "/__control", control)
    payload = {"model": "mock-model", "messages": [], "stream": True}
    if "fail_status" in control:
        with pytest.raises(urllib.error.HTTPError) as error:
            request(server, "/v1/chat/completions", payload)
        assert error.value.code == 429
        return
    _, _, body = request(server, "/v1/chat/completions", payload)
    if "fail_after_n_chunks" in control:
        assert body.count("data:") == 1
    elif control.get("stream_error_event"):
        assert "event: error" in body
    else:
        assert "data:" in body


def test_model_suffix_injection_and_request_recording(server):
    payload = {"model": "mock-model__fail_status=418", "messages": [], "stream": False}
    with pytest.raises(urllib.error.HTTPError) as error:
        request(server, "/v1/chat/completions", payload)
    assert error.value.code == 418
    req = urllib.request.urlopen(server + "/__requests")
    records = json.loads(req.read())['requests']
    assert records[-1]["body"]["model"] == payload["model"]
