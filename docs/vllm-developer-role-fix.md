# vLLM `developer` role 호환 패치

## 증상

Stcode(또는 codex CLI)에서 `qwen3.6-35b-a3b` 같은 vLLM 모델로 turn을 시도하면
`turn/completed` status=failed + 에러:

```
{"error":{"message":"Unexpected message role.","type":"BadRequestError","code":400}}
```

## 원인

OpenAI는 새 Responses API에서 시스템 프롬프트를 **`developer` role** 메시지로 보냅니다.
codex CLI 0.128부터는 `wire_api="chat"`(legacy) 미지원, **Responses API 전용**.
하지만 대부분의 OSS 모델 chat template(Qwen3 포함)은 `system | user | assistant` 만
처리하고 `developer`를 만나면 명시적으로 거절합니다 → 위 에러.

## 해결 — vLLM `--chat-template` 패치

vLLM 서버에 SSH 후, `developer` role을 `system`으로 매핑한 chat template을
별도 파일로 만들고, vLLM 재시작 시 `--chat-template /path/to/file.jinja`로 지정.

### 1. 현재 사용 중인 chat template 추출

```bash
python - <<'PY'
from transformers import AutoTokenizer
t = AutoTokenizer.from_pretrained("/model")  # vLLM이 로드한 model 경로
print(t.chat_template)
PY
```

(또는 모델의 `tokenizer_config.json`에서 `chat_template` 필드 직접 추출)

### 2. 패치 — `developer` 분기를 `system`과 동일하게 처리

추출한 Jinja에 다음 elif 분기를 **추가**합니다 (Qwen3 계열 예시):

```jinja
{%- elif message.role == "developer" %}
<|im_start|>system
{{ message.content }}<|im_end|>
```

또는 더 안전하게, 아예 첫 줄에 alias 매핑:

```jinja
{%- for message in messages %}
{%- set role = "system" if message.role == "developer" else message.role %}
{# 이하 기존 템플릿에서 message.role을 role로 치환 #}
```

### 3. 파일로 저장

```bash
sudo tee /etc/vllm/qwen-developer-role.jinja > /dev/null <<'EOF'
<패치한 템플릿 내용>
EOF
```

### 4. vLLM 재시작

기존 launch command에 `--chat-template /etc/vllm/qwen-developer-role.jinja` 추가:

```bash
vllm serve /model \
  --port 8000 \
  --chat-template /etc/vllm/qwen-developer-role.jinja \
  ...기존 옵션...
```

## 검증

vLLM 재시작 후 로컬에서:

```bash
curl -sS http://100.105.145.6:8000/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model":"qwen3.6-35b-a3b",
    "input":[
      {"role":"developer","content":"You are concise."},
      {"role":"user","content":"hi"}
    ]
  }' | head -c 200
```

`{"id":"resp_…","status":"completed", …}`가 나오면 성공.

이후 Stcode probe:

```bash
cd ~/Documents/GitHub/Stcode
cargo run -q -p stcode-codex --example probe -- "한 줄로 답해줘. 너 누구야?"
```

agentMessage delta가 한 글자씩 흘러나오면 끝.

## 대안 — 서버 패치 없이 갈 때

서버 권한이 없거나 즉시 못 고치는 경우:

1. **다른 vLLM 모델**: chat template이 `developer` role을 원래 처리하는 모델 선택
   (현재 OSS에선 드뭄 — 대부분 Qwen 계열은 같은 문제)
2. **ChatGPT 계정 사용**: codex가 이미 로그인된 OpenAI 호스팅 모델로 임시 전환
   (Stcode 설정 화면에서 provider를 `openai`로)
3. **Reverse-proxy로 role rewrite**: vLLM 앞에 nginx/caddy 두고 요청 본문의
   `"role":"developer"` → `"role":"system"` 치환. 복잡, 비권장.

## Stcode 측 대응

Stcode 코드는 codex의 wire 스펙을 그대로 따르므로 우회 불가. M2 단계의 친화적
에러 메시지에 이 케이스를 추가 — 에러 메시지에 "Unexpected message role" 또는
"role" + "Bad Request" 패턴이 보이면 이 문서 링크 노출 예정.
