//! Generate HTML charts from scenario results.

use crate::report::{DistributionStats, ScenarioResult};
use std::path::Path;

/// Max chart width in pixels (1080p).
const CHART_WIDTH: u32 = 1920;
const CHART_HEIGHT: u32 = 600;
const COMPARISON_HEIGHT: u32 = 400;

/// Generate an HTML file with throughput charts for all scenarios.
pub fn generate_chart(results: &[ScenarioResult], output_path: &Path) -> anyhow::Result<()> {
	let scenario_charts: Vec<String> = results
		.iter()
		.filter(|r| !r.blocks.is_empty())
		.map(render_scenario_chart)
		.collect();

	// Cross-scenario comparison charts (only if multiple scenarios with stats).
	let comparison_html = render_comparison_charts(results);

	if scenario_charts.is_empty() && comparison_html.is_empty() {
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
  .comparison {{ border-left: 4px solid #6366f1; }}
  h1 {{ color: #333; }}
  h2 {{ color: #555; margin: 0 0 12px 0; font-size: 16px; }}
  canvas {{ width: 100% !important; }}
</style>
</head>
<body>
<h1>Stress Test Results</h1>
{comparison}
{scenarios}
</body>
</html>"#,
		comparison = comparison_html,
		scenarios = scenario_charts.join("\n"),
	);

	std::fs::write(output_path, &html)?;
	log::info!("Chart written to {}", output_path.display());
	Ok(())
}

/// Render cross-scenario comparison bar charts for latency, TPS, and throughput.
fn render_comparison_charts(results: &[ScenarioResult]) -> String {
	let with_stats: Vec<_> = results
		.iter()
		.filter(|r| r.latency_ms.is_some() || r.block_tps.is_some())
		.collect();

	if with_stats.is_empty() {
		return String::new();
	}

	let labels: Vec<String> = with_stats
		.iter()
		.map(|r| {
			// Extract a short label from the name.
			r.name
				.split('(')
				.nth(1)
				.and_then(|s| s.split(',').next())
				.unwrap_or(&r.name)
				.to_string()
		})
		.collect();
	let labels_js = labels
		.iter()
		.map(|l| format!("'{l}'"))
		.collect::<Vec<_>>()
		.join(",");

	let mut charts = Vec::new();

	// Latency comparison
	if with_stats.iter().any(|r| r.latency_ms.is_some()) {
		charts.push(render_distribution_bar_chart(
			"latency_cmp",
			"Tx Inclusion Latency (ms) by Variant",
			&labels_js,
			&with_stats
				.iter()
				.map(|r| r.latency_ms.clone())
				.collect::<Vec<_>>(),
		));
	}

	// TPS comparison
	if with_stats.iter().any(|r| r.block_tps.is_some()) {
		charts.push(render_distribution_bar_chart(
			"tps_cmp",
			"Per-Block TPS by Variant",
			&labels_js,
			&with_stats
				.iter()
				.map(|r| r.block_tps.clone())
				.collect::<Vec<_>>(),
		));
	}

	// Throughput comparison
	if with_stats.iter().any(|r| r.block_mbps.is_some()) {
		charts.push(render_distribution_bar_chart(
			"mbps_cmp",
			"Per-Block Throughput (MB/s) by Variant",
			&labels_js,
			&with_stats
				.iter()
				.map(|r| r.block_mbps.clone())
				.collect::<Vec<_>>(),
		));
	}

	charts.join("\n")
}

/// Render a grouped bar chart showing min/avg/P90/P99/max for each variant.
/// Returns empty string if no stats have data.
fn render_distribution_bar_chart(
	id: &str,
	title: &str,
	labels_js: &str,
	stats: &[Option<DistributionStats>],
) -> String {
	// Skip if no scenario has data for this metric.
	if stats.iter().all(|s| s.is_none()) {
		return String::new();
	}

	let extract = |f: fn(&DistributionStats) -> f64| -> String {
		stats
			.iter()
			.map(|s| match s {
				Some(s) => format!("{:.2}", f(s)),
				None => "0".to_string(),
			})
			.collect::<Vec<_>>()
			.join(",")
	};

	let min_data = extract(|s| s.min);
	let avg_data = extract(|s| s.avg);
	let p90_data = extract(|s| s.p90);
	let p99_data = extract(|s| s.p99);
	let max_data = extract(|s| s.max);

	format!(
		r#"<div class="chart-container comparison">
<h2>{title}</h2>
<canvas id="{id}" height="{COMPARISON_HEIGHT}"></canvas>
<script>
new Chart(document.getElementById('{id}'), {{
  type: 'bar',
  data: {{
    labels: [{labels_js}],
    datasets: [
      {{ label: 'Min', data: [{min_data}], backgroundColor: '#22c55e' }},
      {{ label: 'Avg', data: [{avg_data}], backgroundColor: '#3b82f6' }},
      {{ label: 'P90', data: [{p90_data}], backgroundColor: '#f59e0b' }},
      {{ label: 'P99', data: [{p99_data}], backgroundColor: '#ef4444' }},
      {{ label: 'Max', data: [{max_data}], backgroundColor: '#6b7280' }},
    ]
  }},
  options: {{
    responsive: true,
    plugins: {{ legend: {{ position: 'top' }} }},
    scales: {{ y: {{ beginAtZero: true }} }}
  }}
}});
</script>
</div>"#
	)
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

	// Latency summary line
	let latency_info = result
		.latency_ms
		.as_ref()
		.map(|l| {
			format!(
				" | latency: avg {:.0}ms, P99 {:.0}ms",
				l.avg, l.p99
			)
		})
		.unwrap_or_default();

	format!(
		r#"<div class="chart-container">
<h2>{name}{avg_tps}{avg_bps}{latency_info}</h2>
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
