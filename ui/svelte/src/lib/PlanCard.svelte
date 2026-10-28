<script lang="ts">
  import type { ExecutionPlan } from '@agent-runtime/conversation-element/protocol';
  import RichContent from './RichContent.svelte';
  import './plan.css';
  export let plan: ExecutionPlan;
  export let locale = 'en-US';
  let expanded = true;
  let identity = '';
  $: if ((plan.plan_id || plan.created_at || plan.title) !== identity) {
    identity = plan.plan_id || plan.created_at || plan.title;
    expanded = plan.status === 'active';
  }
  $: zh = locale.startsWith('zh');
  $: labels = zh
    ? {
        active: '进行中',
        finished: '已完成',
        canceled: '已取消',
        pending: '待执行',
        in_progress: '进行中',
        completed: '已完成',
        blocked: '受阻',
      }
    : {
        active: 'In progress',
        finished: 'Completed',
        canceled: 'Canceled',
        pending: 'Pending',
        in_progress: 'In progress',
        completed: 'Completed',
        blocked: 'Blocked',
      };
  $: steps = plan.steps ?? [];
  $: completed = steps.filter((step) => step.status === 'completed').length;
</script>

<div class="orbit-svelte-plan">
  <details bind:open={expanded}>
    <summary aria-label={zh ? '执行计划' : 'Execution plan'}
      ><span class="chevron" aria-hidden="true">›</span>
      <span class="heading"
        ><span class="title">{plan.title}</span>
        <div class="meta" role="status">
          {labels[plan.status]}{steps.length
            ? ` · ${completed}/${steps.length}`
            : ''}
        </div></span
      >
      {#if steps.length}<progress
          aria-label={zh ? '计划完成进度' : 'Plan progress'}
          max={steps.length}
          value={completed}
        ></progress>{/if}
    </summary>
    {#if steps.length}<ol>
        {#each steps as step}<li data-status={step.status}>
            <span class="mark" aria-hidden="true"
              >{{
                completed: '✓',
                in_progress: '●',
                pending: '○',
                blocked: '!',
                canceled: '−',
              }[step.status]}</span
            ><span class="text">{step.text}</span><span class="step-status"
              >{labels[step.status]}</span
            >
          </li>{/each}
      </ol>
    {:else}<div class="body">
        <RichContent content={plan.content ?? ''} {locale} widgetsExpired />
      </div>{/if}
  </details>
</div>
