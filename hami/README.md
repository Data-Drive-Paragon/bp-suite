# hami

To install dependencies:

```bash
bun install
```

To run:

```bash
bun run index.ts
```

This project was created using `bun init` in bun v1.3.14. [Bun](https://bun.com) is a fast all-in-one JavaScript runtime.

## Security Configuration

### API Authentication

The `/api/mails/last` endpoint requires authentication when the `API_KEY` environment variable is set.

To enable authentication, set the `API_KEY` environment variable:

```bash
export API_KEY="your-secure-random-key-here"
```

Clients must include the API key in the `X-API-Key` header:

```bash
curl -H "X-API-Key: your-secure-random-key-here" \
  "http://localhost:3000/api/mails/last?email=user@example.com"
```

### API Endpoint Restrictions

The `/api/mails/last` endpoint has the following security restrictions:

1. **Authentication Required**: When `API_KEY` is set, all requests must include a valid `X-API-Key` header
2. **Email Parameter Required**: The `email` query parameter is mandatory to prevent global enumeration
3. **Metadata Only**: The endpoint returns only metadata (id, message_id, sender, recipient, subject, created_at) and excludes sensitive content (plain_body, html_body, raw_payload)

**Important**: Always set the `API_KEY` environment variable in production environments to prevent unauthorized access.
