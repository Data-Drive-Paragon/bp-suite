-- PostgreSQL script to automatically find tables with 'phone', 'username', 'user_id', or 'email' columns,
-- check if they are indexed, and create the index if it doesn't exist.

DO $$
DECLARE
    r RECORD;
    index_name TEXT;
    sql_cmd TEXT;
    created_count INT := 0;
BEGIN
    FOR r IN
        SELECT
            ns.nspname AS schema_name,
            t.relname AS table_name,
            c.attname AS column_name,
            c.attnum AS att_num
        FROM pg_attribute c
        JOIN pg_class t ON c.attrelid = t.oid
        JOIN pg_namespace ns ON t.relnamespace = ns.oid
        WHERE t.relkind = 'r' -- regular tables
          AND ns.nspname NOT IN ('pg_catalog', 'information_schema') -- skip system schemas
          AND c.attname IN ('phone', 'username', 'user_id', 'email')
          AND c.attnum > 0 -- skip system columns
          AND NOT c.attisdropped -- skip dropped columns

          -- Check if an index already exists with this column as the leading column
          AND NOT EXISTS (
              SELECT 1
              FROM pg_index i
              WHERE i.indrelid = t.oid
                AND i.indkey[0] = c.attnum -- column is the first/leading column of the index
          )
    LOOP
        -- Construct standard index name: idx_<table_name>_<column_name>
        index_name := 'idx_' || r.table_name || '_' || r.column_name;

        -- Safely truncate index name if it exceeds PostgreSQL identifier limit (63 chars)
        IF length(index_name) > 63 THEN
            index_name := substring(index_name FROM 1 FOR 50) || '_' || substring(md5(index_name) FROM 1 FOR 10);
        END IF;

        -- Generate the DDL statement
        sql_cmd := format('CREATE INDEX IF NOT EXISTS %I ON %I.%I (%I)',
                          index_name, r.schema_name, r.table_name, r.column_name);

        RAISE NOTICE 'Creating missing index: %', sql_cmd;
        EXECUTE sql_cmd;
        created_count := created_count + 1;
    END LOOP;

    RAISE NOTICE 'Done! Successfully created % indexes.', created_count;
END $$;
