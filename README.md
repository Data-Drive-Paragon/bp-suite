# Big Paragon

Octagon Big-Data cluster management and execution suite.

## API Usage Example

### Execute Staged Query (`POST /api/execute`)

You can execute multi-stage workflows by sending a `POST` request to `/api/execute` with a JSON payload wrapping your stages in a `stages` object:

```json
{
    "stages": {
        "stage1": {
            "source": {
                "type": "pg_stream",
                "raw": "SELECT 1"
            }
        },
        "stage2": {
            "transformer": {
                "type": "lua_script",
                "raw": "return data"
            }
        }
    }
}
```
