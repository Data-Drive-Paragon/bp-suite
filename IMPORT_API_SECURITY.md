# Import API Security Configuration

## Overview

The HTTP Import Server endpoints (`/import` and `/api/import`) now require authentication via API key to prevent unauthorized access to database schema bootstrap and data import operations.

## Configuration

### 1. Set API Key in config.toml

Add or update the `api_key` field in the `[import]` section of your `config.toml`:

```toml
[import]
predicted_hash_policy = "29503:5;"
api_key = "your-secure-random-api-key-here"
```

**Important:** 
- Use a strong, randomly generated API key (recommended: at least 32 characters)
- Keep this key secret and do not commit it to version control
- Rotate the key periodically for enhanced security

### 2. Generate a Secure API Key

You can generate a secure API key using various methods:

**Using OpenSSL:**
```bash
openssl rand -base64 32
```

**Using Python:**
```python
import secrets
print(secrets.token_urlsafe(32))
```

**Using Node.js:**
```javascript
require('crypto').randomBytes(32).toString('base64')
```

## Usage

### Making Authenticated Requests

Include the API key in the `Authorization` header of your HTTP requests:

**Using Bearer token format (recommended):**
```bash
curl -X POST http://localhost:29510/import \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-secure-random-api-key-here" \
  -d '{
    "table_family": "example",
    "version": 1,
    "records": [...]
  }'
```

**Using direct token format:**
```bash
curl -X POST http://localhost:29510/import \
  -H "Content-Type: application/json" \
  -H "Authorization: your-secure-random-api-key-here" \
  -d '{
    "table_family": "example",
    "version": 1,
    "records": [...]
  }'
```

### Response Codes

- **200 OK**: Request successful (with valid API key)
- **401 Unauthorized**: Missing or invalid API key
- **400 Bad Request**: Invalid request format or data errors
- **500 Internal Server Error**: Server-side processing error

## Security Features

1. **API Key Validation**: All import requests must include a valid API key
2. **Constant-Time Comparison**: API key comparison uses constant-time algorithm to prevent timing attacks
3. **Flexible Format Support**: Accepts both "Bearer <token>" and direct token formats
4. **Clear Error Messages**: Provides informative error messages for authentication failures
5. **Startup Warnings**: Server displays authentication status on startup

## Backward Compatibility

If no API key is configured in `config.toml`, the server will:
- Display a warning message on startup
- Log warnings for each unauthenticated request
- **Still allow requests to proceed** (for backward compatibility)

**Security Recommendation:** Always configure an API key in production environments to protect against unauthorized access.

## Migration Guide

### For Existing Deployments

1. Generate a secure API key (see above)
2. Add the `api_key` field to your `config.toml` under `[import]` section
3. Restart the import server
4. Update all client applications to include the API key in the `Authorization` header
5. Verify authentication is working by checking server logs and testing requests

### Testing Authentication

**Test with valid key:**
```bash
curl -X POST http://localhost:29510/import \
  -H "Authorization: Bearer your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"table_family":"test","version":1,"records":[]}'
```

**Test without key (should fail):**
```bash
curl -X POST http://localhost:29510/import \
  -H "Content-Type: application/json" \
  -d '{"table_family":"test","version":1,"records":[]}'
```

Expected response for missing/invalid key:
```json
{
  "success_count": 0,
  "error_count": 1,
  "errors": ["Unauthorized: Missing Authorization header. Please provide API key in Authorization header."]
}
```

## Troubleshooting

### "Unauthorized: Invalid API key"
- Verify the API key in your request matches the one in `config.toml`
- Check for extra whitespace or special characters
- Ensure the key is properly URL-encoded if necessary

### "Unauthorized: Missing Authorization header"
- Add the `Authorization` header to your request
- Verify the header name is spelled correctly (case-sensitive)

### Server shows "Authentication is DISABLED" warning
- Check that `api_key` is present in the `[import]` section of `config.toml`
- Verify the key is not empty
- Restart the server after updating the configuration

## Security Best Practices

1. **Use HTTPS**: Always use HTTPS in production to encrypt API keys in transit
2. **Rotate Keys**: Periodically rotate API keys (e.g., every 90 days)
3. **Restrict Network Access**: Use firewall rules to limit which IPs can access the import server
4. **Monitor Logs**: Regularly review logs for unauthorized access attempts
5. **Secure Storage**: Store `config.toml` with restricted file permissions (e.g., `chmod 600`)
6. **Environment Variables**: Consider using environment variables for sensitive configuration in containerized deployments
