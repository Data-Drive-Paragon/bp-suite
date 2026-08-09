use crate::octagon::Octagon;
use anyhow::{Result, Context};
use bytes::Bytes;
use futures_util::sink::SinkExt;
use console::Style;

struct BenchResult {
    port: u16,
    storage_type: String,
    mean: f64,    // mean time in ms
    stddev: f64,  // stddev in ms
}

pub async fn run_benchmark(octagon: &Octagon, rows: usize) -> Result<()> {
    let bold = Style::new().bold();
    let green = Style::new().green();
    let cyan = Style::new().cyan();
    let red = Style::new().red();

    log::info!("Configured to test with {} rows per database node over 5 runs.", rows);
    println!();

    let mut results = Vec::new();
    let runs = 5;

    for (node_idx, config) in octagon.connections.iter().enumerate() {
        let port = config.port;
        let storage_type = if port == 29501 {
            "CIFS Network"
        } else {
            "Local Disk"
        };
        
        println!("{}: Port {} ({}) - COPY Insertion of {} rows", 
            bold.apply_to(format!("Benchmark {}", node_idx + 1)),
            port,
            storage_type,
            rows
        );

        let client_mutex = octagon.clients.get(&port)
            .context(format!("Failed to find database client for port {}", port))?;

        let mut durations = Vec::with_capacity(runs);

        // Run the benchmark multiple times to gather statistics
        for _run_idx in 1..=runs {
            // --- 0. Setup and Cleanup Old Table ---
            let table_exists = octagon.table_exists("octagon_benchmark_tmp", port).await?;
            {
                let client = client_mutex.lock().await;
                if table_exists {
                    client.execute("DROP TABLE public.octagon_benchmark_tmp CASCADE;", &[]).await?;
                }
                client.execute(
                    "CREATE TABLE public.octagon_benchmark_tmp (
                        id SERIAL PRIMARY KEY,
                        phone TEXT,
                        name TEXT,
                        comment TEXT
                    );",
                    &[]
                ).await.context("Failed to create benchmark temporary table")?;
            }

            // --- 1. Prepare Test Data in Memory ---
            let mut records = Vec::with_capacity(rows);
            for i in 0..rows {
                records.push(vec![
                    format!("7999{:07}", i % 10000000),
                    format!("User_{}", i),
                    format!("This is a standard benchmark row number {} to test PostgreSQL COPY writing throughput and index latency.", i)
                ]);
            }

            // --- 2. Benchmark WRITE (COPY) Speed ---
            let start_write = std::time::Instant::now();
            {
                let mut client = client_mutex.lock().await;
                let tx = client.transaction().await?;
                
                let sql = "COPY public.octagon_benchmark_tmp (phone, name, comment) FROM STDIN";
                let sink = tx.copy_in(sql).await?;
                tokio::pin!(sink);

                let mut buffer = String::with_capacity(records.len() * 128);
                for record in &records {
                    buffer.push_str(&record.join("\t"));
                    buffer.push('\n');
                }

                sink.send(Bytes::from(buffer)).await?;
                sink.finish().await?;
                tx.commit().await?;
            }
            let duration = start_write.elapsed().as_secs_f64() * 1000.0; // convert to ms
            durations.push(duration);

            // --- 3. Cleanup ---
            let table_exists = octagon.table_exists("octagon_benchmark_tmp", port).await?;
            {
                let client = client_mutex.lock().await;
                if table_exists {
                    client.execute("DROP TABLE public.octagon_benchmark_tmp CASCADE;", &[]).await?;
                }
            }
        }

        // Calculate statistics
        let mean = durations.iter().sum::<f64>() / durations.len() as f64;
        let variance = durations.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / durations.len() as f64;
        let stddev = variance.sqrt();
        let min = durations.iter().copied().fold(f64::INFINITY, f64::min);
        let max = durations.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        
        let avg_speed = rows as f64 / (mean / 1000.0); // rows/sec

        // Format speed with comma separator for thousands
        let speed_formatted = if avg_speed >= 1000.0 {
            let thousands = (avg_speed / 1000.0).floor() as u64;
            let remainder = (avg_speed % 1000.0).floor() as u64;
            format!("{},{:03}", thousands, remainder)
        } else {
            format!("{:.0}", avg_speed)
        };

        println!(
            "  Time (mean ± σ):     {} ±   {:.1} ms    [Speed: {} rows/s]",
            green.apply_to(format!("{:.1} ms", mean)),
            stddev,
            speed_formatted
        );
        println!(
            "  Range (min … max):   {:.1} ms … {:.1} ms    {} runs\n",
            min, max, runs
        );

        results.push(BenchResult {
            port,
            storage_type: storage_type.to_string(),
            mean,
            stddev,
        });
    }

    // --- 3. Summary comparison ---
    if results.len() > 1 {
        // Sort results so that the fastest node (lowest mean execution time) is first
        results.sort_by(|a, b| a.mean.partial_cmp(&b.mean).unwrap());

        println!("{}", bold.apply_to("Summary"));
        println!("  {}", cyan.apply_to(format!("Port {} ({})", results[0].port, results[0].storage_type)));

        for i in 1..results.len() {
            let speedup = results[i].mean / results[0].mean;
            // Standard deviation propagation formula for division (X = A / B)
            // σ_X = X * sqrt((σ_A / A)^2 + (σ_B / B)^2)
            let rel_stddev_a = results[i].stddev / results[i].mean;
            let rel_stddev_b = results[0].stddev / results[0].mean;
            let speedup_stddev = speedup * (rel_stddev_a.powi(2) + rel_stddev_b.powi(2)).sqrt();

            println!(
                "    {} ± {:.2} times faster than {}",
                green.apply_to(format!("{:.2}", speedup)),
                speedup_stddev,
                red.apply_to(format!("Port {} ({})", results[i].port, results[i].storage_type))
            );
        }
        println!();
    }

    Ok(())
}
