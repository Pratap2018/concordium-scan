<template>
	<Tooltip
		v-if="
			props.selfSuspended === null &&
			props.inactiveSuspended === null &&
			props.primedForSuspension === null
		"
		:text="`Validator is active.`"
	>
		<span class="numerical change" style="color: hsl(var(--color-interactive))">
			Active
		</span>
	</Tooltip>
	<Tooltip
		v-else-if="
			props.primedForSuspension !== null &&
			props.selfSuspended === null &&
			props.inactiveSuspended === null
		"
		:text="`Validator will be suspended on the next pay day.`"
	>
		<span class="numerical change" style="color: #ffc600">
			Primed
			<WarningIcon class="h-4 align-middle" />
		</span>
	</Tooltip>
	<Tooltip
		v-else-if="props.inactiveSuspended !== null"
		:text="`Validator is suspended due to inactivity.`"
	>
		<span class="numerical change" style="color: #ffc600">
			Suspended
			<WarningIcon class="h-4 align-middle" />
		</span>
	</Tooltip>
	<Tooltip
		v-else-if="props.selfSuspended !== null"
		:text="`Validator is suspended because the validator sent a self-suspending transaction.`"
	>
		<span class="numerical change" style="color: #ffc600">
			Suspended
			<WarningIcon class="h-4 align-middle" />
		</span>
	</Tooltip>
</template>
<script lang="ts" setup>
import WarningIcon from '../icons/WarningIcon.vue'
import Tooltip from '../atoms/Tooltip.vue'

type Props = {
	selfSuspended: number | null
	inactiveSuspended: number | null
	primedForSuspension: number | null
}
const props = defineProps<Props>()
</script>
