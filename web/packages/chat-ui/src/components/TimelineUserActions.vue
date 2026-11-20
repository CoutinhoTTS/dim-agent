<script setup lang="ts">
import { computed } from 'vue'
import type { ChatUiTimelineUserActionsProps } from './user-message-actions'

defineOptions({
  name: 'TimelineUserActions',
})

const props = withDefaults(defineProps<ChatUiTimelineUserActionsProps>(), {
  branchDisabled: false,
  actionPending: false,
  copied: false,
})

const emit = defineEmits<{
  copy: []
  edit: []
  withdraw: []
  fork: []
  branch: []
}>()

const buttonComponent = computed(() => props.buttonComponent ?? 'button')

function handleWithdraw(): void {
  emit('withdraw')
}

function handleCopy(): void {
  emit('copy')
}

function handleEdit(): void {
  if (props.actionPending) return
  emit('edit')
}

function handleFork(): void {
  if (props.actionPending) return
  emit('fork')
}

function handleBranch(): void {
  if (props.branchDisabled || props.actionPending) return
  emit('branch')
}
</script>

<template>
  <component :is="props.tooltipProviderComponent">
    <div
      data-testid="timeline-user-actions"
      class="flex h-7 items-center gap-1 text-muted-foreground opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100 group-focus-within:opacity-100"
    >
      <span
        data-testid="timeline-user-sent-time"
        class="mr-1 shrink-0 text-sm tabular-nums leading-none"
      >
        {{ props.timeText }}
      </span>
      <component :is="props.tooltipComponent" v-if="props.canWithdraw">
        <component :is="props.tooltipTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="ghost"
            size="icon-sm"
            data-testid="timeline-user-withdraw"
            class="h-7 w-7 rounded-lg"
            @click="handleWithdraw"
          >
            <component :is="props.icons.withdraw" class="h-3.5 w-3.5" />
          </component>
        </component>
        <component :is="props.tooltipContentComponent" side="top">{{ props.labels.withdraw }}</component>
      </component>
      <component :is="props.tooltipComponent">
        <component :is="props.tooltipTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="ghost"
            size="icon-sm"
            data-testid="timeline-user-copy"
            class="h-7 w-7 rounded-lg"
            @click="handleCopy"
          >
            <component
              :is="props.copied ? props.icons.check : props.icons.copy"
              class="h-3.5 w-3.5"
              :class="{ 'text-green-600': props.copied }"
              :data-testid="props.copied ? 'copy-success-icon' : undefined"
            />
          </component>
        </component>
        <component :is="props.tooltipContentComponent" side="top">{{ props.labels.copy }}</component>
      </component>
      <component :is="props.tooltipComponent" v-if="props.canEdit">
        <component :is="props.tooltipTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="ghost"
            size="icon-sm"
            data-testid="timeline-user-edit"
            :disabled="props.actionPending"
            class="h-7 w-7 rounded-lg"
            @click="handleEdit"
          >
            <component :is="props.icons.edit" class="h-3.5 w-3.5" />
          </component>
        </component>
        <component :is="props.tooltipContentComponent" side="top">{{ props.labels.edit }}</component>
      </component>
      <component :is="props.tooltipComponent" v-if="props.canFork">
        <component :is="props.tooltipTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="ghost"
            size="icon-sm"
            data-testid="timeline-user-fork"
            :disabled="props.actionPending"
            class="h-7 w-7 rounded-lg"
            @click="handleFork"
          >
            <component :is="props.icons.fork" class="h-3.5 w-3.5" />
          </component>
        </component>
        <component :is="props.tooltipContentComponent" side="top">{{ props.labels.fork }}</component>
      </component>
      <component :is="props.tooltipComponent" v-if="props.canBranch">
        <component :is="props.tooltipTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="ghost"
            size="icon-sm"
            data-testid="timeline-user-branch"
            :disabled="props.branchDisabled || props.actionPending"
            class="h-7 w-7 rounded-lg"
            @click="handleBranch"
          >
            <component :is="props.icons.branch" class="h-3.5 w-3.5" />
          </component>
        </component>
        <component :is="props.tooltipContentComponent" side="top">{{ props.labels.branch }}</component>
      </component>
    </div>
  </component>
</template>
