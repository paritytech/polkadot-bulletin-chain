//! Generate HTML charts from scenario results.

use crate::report::ScenarioResult;
use std::path::Path;

/// Max chart width in pixels (1080p).
const CHART_WIDTH: u32 = 1920;
const CHART_HEIGHT: u32 = 600;

/// Generate an HTML file with throughput charts for all scenarios.
pub fn generate_chart(results: &[ScenarioResult], output_path: &Path) -> anyhow::Result<()> {
	let charts_html: Vec<String> = results
		.iter()
		.filter(|r| !r.blocks.is_empty())
		.map(render_scenario_chart)
		.collect();

	if charts_html.is_empty() {
		log::warn!("No block data to chart");
		return Ok(());
	}

	let html = format!(
		r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Stress Test Results</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4"></script>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
         max-width: {CHART_WIDTH}px; margin: 0 auto; padding: 20px; background: #fafafa; }}
  .chart-container {{ background: white; border-radius: 8px; padding: 16px;
                      margin-bottom: 24px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
  h1 {{ color: #333; }}
  h2 {{ color: #555; margin: 0 0 12px 0; font-size: 16px; }}
  canvas {{ width: 100% !important; }}
</style>
</head>
<body>
<h1>Stress Test Results</h1>
{}
</body>
</html>"#,
		charts_html.join("\n")
	);

	std::fs::write(output_path, &html)?;
	log::info!("Chart written to {}", output_path.display());
	Ok(())
}

fn render_scenario_chart(result: &ScenarioResult) -> String {
	let blocks: Vec<_> = result.blocks.iter().filter(|b| !b.prefill).collect();
	if blocks.is_empty() {
		return String::new();
	}

	let id = result
		.name
		.replace(|c: char| !c.is_alphanumeric(), "_")
		.to_lowercase();

	// X-axis: block numbers
	let labels: Vec<String> = blocks.iter().map(|b| b.number.to_string()).collect();

	// Y1: payload bytes per block (MB)
	let payload_data: Vec<String> = blocks
		.iter()
		.map(|b| format!("{:.3}", b.payload_bytes as f64 / (1024.0 * 1024.0)))
		.collect();

	// Y2: tx count per block
	let tx_data: Vec<String> = blocks.iter().map(|b| b.tx_count.to_string()).collect();

	let avg_tps = if result.throughput_tps > 0.0 {
		format!(" | {:.1} TPS", result.throughput_tps)
	} else {
		String::new()
	};
	let avg_bps = if result.throughput_bytes_per_sec > 0.0 {
		format!(
			" | {:.2} MB/s",
			result.throughput_bytes_per_sec / (1024.0 * 1024.0)
		)
	} else {
		String::new()
	};

	format!(
		r#"<div class="chart-container">
<h2>{name}{avg_tps}{avg_bps}</h2>
<canvas id="{id}" height="{CHART_HEIGHT}"></canvas>
<script>
new Chart(document.getElementById('{id}'), {{
  type: 'line',
  data: {{
    labels: [{labels}],
    datasets: [
      {{
        label: 'Block Size (MB)',
        data: [{payload_data}],
        borderColor: '#2563eb',
        backgroundColor: 'rgba(37, 99, 235, 0.1)',
        fill: true,
        yAxisID: 'y',
        tension: 0.2,
        pointRadius: 1,
      }},
      {{
        label: 'Transactions',
        data: [{tx_data}],
        borderColor: '#dc2626',
        backgroundColor: 'rgba(220, 38, 38, 0.1)',
        fill: false,
        yAxisID: 'y1',
        tension: 0.2,
        pointRadius: 1,
      }}
    ]
  }},
  options: {{
    responsive: true,
    interaction: {{ mode: 'index', intersect: false }},
    plugins: {{
      legend: {{ position: 'top' }},
    }},
    scales: {{
      x: {{
        title: {{ display: true, text: 'Block Number' }},
        ticks: {{ maxTicksLimit: 30 }},
      }},
      y: {{
        type: 'linear',
        position: 'left',
        title: {{ display: true, text: 'Block Size (MB)' }},
        beginAtZero: true,
      }},
      y1: {{
        type: 'linear',
        position: 'right',
        title: {{ display: true, text: 'Transactions' }},
        beginAtZero: true,
        grid: {{ drawOnChartArea: false }},
      }}
    }}
  }}
}});
</script>
</div>"#,
		name = result.name,
		id = id,
		labels = labels.iter().map(|l| format!("'{l}'")).collect::<Vec<_>>().join(","),
		payload_data = payload_data.join(","),
		tx_data = tx_data.join(","),
	)
}
