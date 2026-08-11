#!/bin/bash
# Test script for Import API authentication
# This script demonstrates the security fix for unauthenticated import endpoints

set -e

# Configuration
SERVER_URL="${SERVER_URL:-http://localhost:29510}"
API_KEY="${API_KEY:-}"

echo "=========================================="
echo "Import API Authentication Test"
echo "=========================================="
echo ""

# Test 1: Request without authentication (should fail if API key is configured)
echo "Test 1: Request without Authorization header"
echo "Expected: 401 Unauthorized (if API key configured) or 400 Bad Request (if not configured)"
echo ""
curl -X POST "${SERVER_URL}/import" \
  -H "Content-Type: application/json" \
  -d '{
    "table_family": "test",
    "version": 1,
    "records": []
  }' \
  -w "\nHTTP Status: %{http_code}\n" \
  -s || true

echo ""
echo "=========================================="
echo ""

# Test 2: Request with invalid API key (should fail if API key is configured)
echo "Test 2: Request with invalid API key"
echo "Expected: 401 Unauthorized (if API key configured)"
echo ""
curl -X POST "${SERVER_URL}/import" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer invalid-key-12345" \
  -d '{
    "table_family": "test",
    "version": 1,
    "records": []
  }' \
  -w "\nHTTP Status: %{http_code}\n" \
  -s || true

echo ""
echo "=========================================="
echo ""

# Test 3: Request with valid API key (should succeed if API key is configured)
if [ -n "$API_KEY" ]; then
  echo "Test 3: Request with valid API key"
  echo "Expected: 200 OK or 400 Bad Request (depending on data validity)"
  echo ""
  curl -X POST "${SERVER_URL}/import" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${API_KEY}" \
    -d '{
      "table_family": "test",
      "version": 1,
      "records": []
    }' \
    -w "\nHTTP Status: %{http_code}\n" \
    -s || true
else
  echo "Test 3: Skipped (no API_KEY environment variable set)"
  echo "To test with valid API key, run:"
  echo "  API_KEY=your-api-key $0"
fi

echo ""
echo "=========================================="
echo "Test Summary"
echo "=========================================="
echo ""
echo "If API key is configured in config.toml:"
echo "  - Test 1 should return 401 Unauthorized"
echo "  - Test 2 should return 401 Unauthorized"
echo "  - Test 3 should return 200 OK or 400 (data validation)"
echo ""
echo "If API key is NOT configured:"
echo "  - All tests may succeed (backward compatibility mode)"
echo "  - Server logs should show warnings about missing API key"
echo ""
echo "Security Recommendation:"
echo "  Always configure an API key in production environments!"
echo ""
