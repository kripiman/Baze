// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.domain

import cl.baze.core.model.Hazard
import kotlinx.datetime.Clock

/**
 * Filtra reportes vigentes descartando aquellos que hayan superado su expiresAt.
 */
class FilterActiveHazardsUseCase(
    private val clock: Clock = Clock.System
) {
    fun execute(hazards: List<Hazard>): List<Hazard> {
        val now = clock.now()
        return hazards.filter { it.expiresAt > now }
    }
}
