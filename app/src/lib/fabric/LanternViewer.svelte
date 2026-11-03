<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    fetchLanternGraph,
    applySteering,
    type LanternGraph,
    type LanternNode,
    type SteeringAction,
    type SteeringReport,
  } from './lantern';

  let { taskId }: { taskId: string } = $props();

  let graph = $state<LanternGraph | null>(null);
  let selectedNode = $state<LanternNode | null>(null);
  let timelineIndex = $state<number>(-1);
  let pollActive = $state<boolean>(false);
  let loading = $state<boolean>(false);
  let errorMessage = $state<string | null>(null);
  let report = $state<SteeringReport | null>(null);
  let constraintText = $state<string>('');
  let redirectText = $state<string>('');
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  async function loadGraph() {
    loading = true;
    errorMessage = null;
    try {
      const g = await fetchLanternGraph(taskId);
      graph = g;
      if (timelineIndex === -1 && g.timeline.length > 0) {
        timelineIndex = g.timeline.length - 1;
      }
      if (selectedNode && g.nodes[selectedNode.id]) {
        selectedNode = g.nodes[selectedNode.id];
      }
    } catch (e) {
      errorMessage = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  function togglePoll() {
    pollActive = !pollActive;
    if (pollActive) {
      pollInterval = setInterval(loadGraph, 2000);
    } else if (pollInterval) {
      clearInterval(pollInterval);
      pollInterval = null;
    }
  }

  async function steer(action: SteeringAction) {
    loading = true;
    errorMessage = null;
    report = null;
    try {
      const rep = await applySteering(taskId, action);
      report = rep;
      await loadGraph();
    } catch (e) {
      errorMessage = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadGraph();
  });

  onDestroy(() => {
    if (pollInterval) {
      clearInterval(pollInterval);
    }
  });

  const visibleNodes = $derived(() => {
    if (!graph) return [];
    const entries = Object.values(graph.nodes);
    if (timelineIndex < 0 || timelineIndex >= graph.timeline.length) {
      return entries;
    }
    const cutoffTime = graph.timeline[timelineIndex].timestamp;
    return entries.filter((n) => n.timestamp <= cutoffTime);
  });

  function getStatusClass(status: string): string {
    switch (status) {
      case 'resolved':
        return 'status-resolved';
      case 'active':
        return 'status-active';
      case 'stale':
      case 'eliminated':
        return 'status-stale';
      case 'pruned':
        return 'status-pruned';
      default:
        return '';
    }
  }

  function getKindIcon(kind: string): string {
    switch (kind) {
      case 'goal':
        return '🎯';
      case 'question':
        return '❓';
      case 'branch':
        return '🌿';
      case 'evidence':
        return '🔍';
      case 'hypothesis':
        return '💡';
      case 'assumption':
        return '⚠️';
      case 'decision':
        return '⚖️';
      case 'gate':
        return '🚪';
      case 'action':
        return '⚡';
      case 'outcome':
        return '🏁';
      case 'steering':
        return '🧭';
      default:
        return '•';
    }
  }
</script>

<div class="lantern-viewer">
  <div class="lantern-header">
    <div class="header-left">
      <span class="lantern-title">🏮 Lantern Decision Provenance</span>
      {#if loading}
        <span class="loading-tag">Loading…</span>
      {/if}
    </div>
    <div class="header-actions">
      <button class="btn btn-secondary" onclick={loadGraph} disabled={loading}>Refresh</button>
      <button
        class="btn {pollActive ? 'btn-active' : 'btn-secondary'}"
        onclick={togglePoll}
      >
        {pollActive ? 'Live Polling (On)' : 'Live Polling (Off)'}
      </button>
    </div>
  </div>

  {#if errorMessage}
    <div class="alert alert-error">
      <span>{errorMessage}</span>
    </div>
  {/if}

  {#if report}
    <div class="alert alert-info">
      <strong>Steering Applied ({report.action_type}):</strong> {report.details}
      {#if report.affected_nodes.length > 0}
        <div>Affected: {report.affected_nodes.join(', ')}</div>
      {/if}
      {#each report.warnings as w}
        <div class="warning-text">⚠️ {w}</div>
      {/each}
    </div>
  {/if}

  {#if graph && graph.timeline.length > 0}
    <div class="lantern-timeline-card">
      <div class="timeline-header">
        <span>Lantern Width & Exploration Scrubber ({graph.timeline.length} events)</span>
        <span class="timeline-step">
          Step {timelineIndex + 1} / {graph.timeline.length} (Width: {graph.timeline[Math.max(0, timelineIndex)]?.width ?? 0})
        </span>
      </div>
      <div class="timeline-chart">
        {#each graph.timeline as entry, i}
          <button
            type="button"
            class="chart-bar {i === timelineIndex ? 'active-bar' : ''}"
            style="height: {Math.max(12, entry.width * 20)}px;"
            aria-label="{entry.timestamp} | Width: {entry.width}"
            title="{entry.timestamp} | Width: {entry.width}"
            onclick={() => (timelineIndex = i)}
          ></button>
        {/each}
      </div>
      <input
        type="range"
        min="0"
        max={graph.timeline.length - 1}
        bind:value={timelineIndex}
        class="timeline-slider"
      />
    </div>
  {/if}

  <div class="lantern-body">
    <div class="graph-tree">
      {#if graph}
        {#each visibleNodes() as node (node.id)}
          <div
            role="button"
            tabindex="0"
            class="node-card {getStatusClass(node.status)} {selectedNode?.id === node.id ? 'selected' : ''}"
            style="margin-left: {node.depth * 20}px;"
            onclick={() => (selectedNode = node)}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                selectedNode = node;
              }
            }}
          >
            <div class="node-main">
              <span class="node-icon">{getKindIcon(node.kind)}</span>
              <div class="node-label-group">
                <span class="node-label">{node.label}</span>
                <span class="node-meta">{node.kind} · {node.status}</span>
              </div>
            </div>

            <div class="node-quick-actions">
              {#if node.kind === 'assumption'}
                <button
                  type="button"
                  class="btn btn-sm btn-danger"
                  onclick={(e) => {
                    e.stopPropagation();
                    steer({ action: 'reject_assumption', assumption_id: node.data.assumption_id as string || node.id });
                  }}
                >
                  Reject
                </button>
              {:else if node.kind === 'question' && node.status !== 'resolved'}
                <button
                  type="button"
                  class="btn btn-sm btn-secondary"
                  onclick={(e) => {
                    e.stopPropagation();
                    steer({ action: 'explore_further', question_id: node.data.question_id as string || node.id });
                  }}
                >
                  Explore Further
                </button>
              {:else if node.kind === 'branch' && node.status !== 'pruned'}
                <button
                  type="button"
                  class="btn btn-sm btn-secondary"
                  onclick={(e) => {
                    e.stopPropagation();
                    steer({
                      action: 'prune_branch',
                      branch_id: node.data.branch_id as string || node.id,
                      reason: 'pruned by user via lantern viewer',
                    });
                  }}
                >
                  Prune
                </button>
              {:else if node.kind === 'gate'}
                <button
                  type="button"
                  class="btn btn-sm btn-tint"
                  onclick={(e) => {
                    e.stopPropagation();
                    steer({
                      action: 'authorize_despite_gate',
                      gate_id: node.id,
                      reason: 'user risk acceptance via lantern viewer',
                    });
                  }}
                >
                  Authorize Despite Gate
                </button>
              {/if}
            </div>
          </div>
        {/each}
      {:else if !loading}
        <div class="empty-state">No Lantern run provenance found for this task yet.</div>
      {/if}
    </div>

    {#if selectedNode}
      <div class="inspection-panel">
        <div class="inspection-head">
          <h3>Node Inspection: {selectedNode.id}</h3>
          <button class="btn btn-sm btn-secondary" onclick={() => (selectedNode = null)}>✕</button>
        </div>
        <div class="inspection-content">
          <div class="prop-row">
            <span class="prop-name">Kind:</span>
            <span>{selectedNode.kind}</span>
          </div>
          <div class="prop-row">
            <span class="prop-name">Status:</span>
            <span class="status-badge {getStatusClass(selectedNode.status)}">{selectedNode.status}</span>
          </div>
          <div class="prop-row">
            <span class="prop-name">Timestamp:</span>
            <span>{selectedNode.timestamp}</span>
          </div>
          <div class="prop-row">
            <span class="prop-name">Label:</span>
            <span>{selectedNode.label}</span>
          </div>

          {#if selectedNode.data}
            <div class="data-section">
              <h4>Structured Payload</h4>
              <pre class="json-dump">{JSON.stringify(selectedNode.data, null, 2)}</pre>
            </div>
          {/if}

          <div class="steering-panel">
            <h4>Steer Investigation</h4>
            {#if selectedNode.kind === 'assumption'}
              <button
                class="btn btn-danger w-full"
                onclick={() =>
                  steer({
                    action: 'reject_assumption',
                    assumption_id: selectedNode?.data?.assumption_id as string || selectedNode!.id,
                  })}
              >
                Reject Assumption & Invalidate Dependents
              </button>
            {/if}

            <div class="steering-inputs">
              <div class="input-group">
                <input
                  type="text"
                  placeholder="Add constraint to guide exploration..."
                  bind:value={constraintText}
                  class="text-input"
                />
                <button
                  class="btn btn-secondary"
                  disabled={!constraintText.trim()}
                  onclick={() => {
                    steer({ action: 'add_constraint', text: constraintText.trim() });
                    constraintText = '';
                  }}
                >
                  Add Constraint
                </button>
              </div>

              <div class="input-group">
                <input
                  type="text"
                  placeholder="Redirect exploration with new prompt..."
                  bind:value={redirectText}
                  class="text-input"
                />
                <button
                  class="btn btn-secondary"
                  disabled={!redirectText.trim()}
                  onclick={() => {
                    steer({ action: 'redirect', instructions: redirectText.trim() });
                    redirectText = '';
                  }}
                >
                  Redirect
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .lantern-viewer {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px;
    background: var(--surface-bg, #1a1a1a);
    border-radius: 8px;
    color: var(--text-color, #e0e0e0);
    font-family: inherit;
  }

  .lantern-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .lantern-title {
    font-size: 1.15rem;
    font-weight: 600;
  }

  .loading-tag {
    margin-left: 8px;
    font-size: 0.85rem;
    color: #999;
  }

  .header-actions {
    display: flex;
    gap: 8px;
  }

  .alert {
    padding: 10px 14px;
    border-radius: 6px;
    font-size: 0.9rem;
  }

  .alert-error {
    background: #4a1515;
    color: #ff9e9e;
  }

  .alert-info {
    background: #1c2e42;
    color: #8ec3ff;
  }

  .warning-text {
    color: #ffd074;
    margin-top: 4px;
  }

  .lantern-timeline-card {
    background: rgba(255, 255, 255, 0.04);
    border-radius: 6px;
    padding: 12px;
  }

  .timeline-header {
    display: flex;
    justify-content: space-between;
    font-size: 0.85rem;
    color: #aaa;
    margin-bottom: 8px;
  }

  .timeline-chart {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    height: 60px;
    background: rgba(0, 0, 0, 0.2);
    border-radius: 4px;
    padding: 4px;
    overflow-x: auto;
  }

  .chart-bar {
    flex: 1;
    min-width: 8px;
    background: #4a779d;
    border-radius: 2px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .chart-bar:hover {
    background: #6fa8dc;
  }

  .active-bar {
    background: #ffaa33 !important;
  }

  .timeline-slider {
    width: 100%;
    margin-top: 8px;
  }

  .lantern-body {
    display: flex;
    gap: 16px;
    min-height: 380px;
  }

  .graph-tree {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 550px;
    overflow-y: auto;
  }

  .node-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.05);
    border-left: 4px solid #666;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }

  .node-card:hover {
    background: rgba(255, 255, 255, 0.09);
  }

  .node-card.selected {
    outline: 2px solid #5d9cec;
  }

  .status-resolved {
    border-left-color: #38b000 !important;
  }

  .status-active {
    border-left-color: #ffd000 !important;
  }

  .status-stale {
    border-left-color: #d90429 !important;
  }

  .status-pruned {
    border-left-color: #6c757d !important;
    opacity: 0.6;
  }

  .node-main {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .node-icon {
    font-size: 1.2rem;
  }

  .node-label-group {
    display: flex;
    flex-direction: column;
  }

  .node-label {
    font-weight: 500;
    font-size: 0.95rem;
  }

  .node-meta {
    font-size: 0.75rem;
    color: #888;
    text-transform: capitalize;
  }

  .inspection-panel {
    width: 320px;
    background: rgba(0, 0, 0, 0.3);
    border-radius: 6px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-height: 550px;
    overflow-y: auto;
  }

  .inspection-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
    padding-bottom: 8px;
  }

  .inspection-head h3 {
    margin: 0;
    font-size: 0.95rem;
  }

  .prop-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.85rem;
    margin-bottom: 6px;
  }

  .prop-name {
    color: #888;
  }

  .data-section h4,
  .steering-panel h4 {
    margin: 8px 0 4px 0;
    font-size: 0.85rem;
    color: #aaa;
  }

  .json-dump {
    background: #111;
    padding: 8px;
    border-radius: 4px;
    font-size: 0.75rem;
    max-height: 180px;
    overflow: auto;
  }

  .steering-inputs {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .input-group {
    display: flex;
    gap: 4px;
  }

  .text-input {
    flex: 1;
    background: #222;
    border: 1px solid #444;
    border-radius: 4px;
    padding: 6px 8px;
    color: #fff;
    font-size: 0.8rem;
  }

  .btn {
    border: none;
    border-radius: 4px;
    padding: 6px 12px;
    cursor: pointer;
    font-size: 0.85rem;
    transition: opacity 0.15s;
  }

  .btn:hover {
    opacity: 0.85;
  }

  .btn-sm {
    padding: 4px 8px;
    font-size: 0.75rem;
  }

  .btn-secondary {
    background: #333;
    color: #eee;
  }

  .btn-active {
    background: #2b7a4b;
    color: #fff;
  }

  .btn-danger {
    background: #9b2226;
    color: #fff;
  }

  .btn-tint {
    background: #005f73;
    color: #fff;
  }

  .w-full {
    width: 100%;
  }

  .empty-state {
    padding: 40px;
    text-align: center;
    color: #777;
    font-size: 0.9rem;
  }
</style>
