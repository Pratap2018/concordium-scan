import type { PageInfo } from '~/types/generated'

export const PAGE_SIZE = 25
export const PAGE_SIZE_SMALL = 10

export type PaginationTarget = 'first' | 'previous' | 'next' | 'last'

/**
 * CURRIED: Navigation handler to modify query variables
 * @param { PageInfo } pageInfo - Most recent pageInfo
 * @param { PaginationTarget } target - The target (e.g. "next")
 */
export type PaginationGoToPage = (
	pageInfo: PageInfo
) => (target: PaginationTarget) => void

/**
 * Hook to control pagination state and actions
 * Returns query variables and a curried navigation handler
 */
export const usePagination = (
	{ pageSize }: { pageSize?: number } = { pageSize: PAGE_SIZE }
) => {
	const after = ref<PageInfo['endCursor']>(undefined)
	const before = ref<PageInfo['startCursor']>(undefined)
	const first = ref<number | undefined>(pageSize)
	const last = ref<number | undefined>(undefined)

	/**
	 * CURRIED: Navigation handler to modify query variables
	 * @param { PageInfo } pageInfo - Most recent pageInfo
	 * @param { PaginationTarget } target - The target (e.g. "next")
	 */
	const goToPage = (pageInfo: PageInfo) => (target: PaginationTarget) => {
		after.value = undefined
		before.value = undefined
		first.value = undefined
		last.value = undefined

		if (target === 'first') {
			first.value = pageSize
		} else if (target === 'previous' && pageInfo.hasPreviousPage) {
			before.value = pageInfo.startCursor
			last.value = pageSize
		} else if (target === 'next' && pageInfo.endCursor) {
			after.value = pageInfo.endCursor
			first.value = pageSize
		} else if (target === 'last' && pageInfo.hasNextPage) {
			last.value = pageSize
		} else {
			console.error('Incorrect pagination arguments:', { target, ...pageInfo })
		}
	}

	// Reset cursors and pagination when changing sorting/filter
	const resetPagination = () => {
		after.value = undefined
		before.value = undefined
		first.value = pageSize
		last.value = undefined
	}

	return {
		after,
		before,
		first,
		last,
		goToPage,
		resetPagination,
	}
}
