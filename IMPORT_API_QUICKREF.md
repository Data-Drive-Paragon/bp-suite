# Import API Authentication - Quick Reference

## Setup (One-Time)

### 1. Generate API Key
```bash
openssl rand -base64 32
```

### 2. Configure in config.toml
```toml
[import]
api_key = "paste-generated-key-here"
```

### 3. Restart Server
```bash
# The server will show: 🔒 Authentication: ENABLED
```

## Usage

### cURL
```bash
curl -X POST http://localhost:29510/import \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d @request.json
```

### Python
```python
import requests

headers = {
    'Authorization': 'Bearer YOUR_API_KEY',
    'Content-Type': 'application/json'
}

data = {
    'table_family': 'example',
    'version': 1,
    'records': [...]
}

response = requests.post(
    'http://localhost:29510/import',
    headers=headers,
    json=data
)
```

### JavaScript/Node.js
```javascript
const axios = require('axios');

const response = await axios.post(
  'http://localhost:29510/import',
  {
    table_family: 'example',
    version: 1,
    records: [...]
  },
  {
    headers: {
      'Authorization': 'Bearer YOUR_API_KEY',
      'Content-Type': 'application/json'
    }
  }
);
```

### Go
```go
import (
    "bytes"
    "net/http"
)

req, _ := http.NewRequest("POST", "http://localhost:29510/import", bytes.NewBuffer(jsonData))
req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
req.Header.Set("Content-Type", "application/json")

client := &http.Client{}
resp, err := client.Do(req)
```

## Error Responses

### 401 Unauthorized - Missing Header
```json
{
  "success_count": 0,
  "error_count": 1,
  "errors": ["Unauthorized: Missing Authorization header. Please provide API key in Authorization header."]
}
```

### 401 Unauthorized - Invalid Key
```json
{
  "success_count": 0,
  "error_count": 1,
  "errors": ["Unauthorized: Invalid API key"]
}
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Server shows "Authentication is DISABLED" | Add `api_key` to `[import]` section in config.toml |
| 401 Invalid API key | Verify key matches config.toml exactly (no extra spaces) |
| 401 Missing header | Add `Authorization: Bearer <key>` header to request |
| Still getting 401 | Restart server after updating config.toml |

## Security Checklist

- [ ] API key configured in config.toml
- [ ] API key is at least 32 characters
- [ ] API key stored securely (not in version control)
- [ ] Using HTTPS in production
- [ ] Server shows "🔒 Authentication: ENABLED" on startup
- [ ] All clients updated with API key
- [ ] Firewall rules restrict access to import endpoints
- [ ] Monitoring set up for authentication failures

## Quick Test

```bash
# Should fail (401)
curl -X POST http://localhost:29510/import \
  -H "Content-Type: application/json" \
  -d '{"table_family":"test","version":1,"records":[]}'

# Should succeed (with valid key)
curl -X POST http://localhost:29510/import \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"table_family":"test","version":1,"records":[]}'
```

## Support

For detailed documentation, see: `IMPORT_API_SECURITY.md`
For patch details, see: `SECURITY_PATCH_SUMMARY.md`
