# Big Paragon

Octagon Big-Data cluster management and execution suite.

## API Usage Example

### Execute Staged Query (`POST /api/execute`)

You can execute multi-stage workflows by sending a `POST` request to `/api/execute` with a JSON payload where `stages` is a list (array) of stage objects, each describing its source and/or transformer (with data flowing sequentially between stages):

```json
{
    "stages": [
        {
            "source": {
                "type": "pg_stream",
                "raw": "SELECT 1"
            },
            "transformer": {
                "type": "lua_script",
                "raw": "return data"
            }
        },
        {
            "source": {
                "type": "pg_stream",
                "raw": "SELECT 2"
            },
            "transformer": {
                "type": "lua_script",
                "raw": "return data + 1"
            }
        }
    ]
}
```
