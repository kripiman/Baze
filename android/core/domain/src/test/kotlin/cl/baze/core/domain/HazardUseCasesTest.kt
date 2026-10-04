// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.domain

import cl.baze.core.model.Coordinates
import cl.baze.core.model.Hazard
import cl.baze.core.model.HazardCategory
import cl.baze.core.model.HazardStatus
import kotlinx.datetime.Clock
import kotlinx.datetime.Instant
import org.junit.Assert.assertEquals
import org.junit.Test
import kotlin.time.Duration.Companion.hours

class HazardUseCasesTest {
    private val now = Instant.parse("2026-10-04T12:00:00Z")
    private val clock = object : Clock {
        override fun now(): Instant = now
    }

    private fun hazard(id: String, at: Coordinates, expiresIn: kotlin.time.Duration = 5.hours) = Hazard(
        id = id,
        category = HazardCategory.POTHOLE,
        status = HazardStatus.CONFIRMED,
        upvotes = 1,
        downvotes = 0,
        location = at,
        createdAt = now - 1.hours,
        expiresAt = now + expiresIn,
    )

    @Test
    fun `expired reports are dropped`() {
        val here = Coordinates(-70.65, -33.45)
        val live = hazard("live", here)
        val expired = hazard("expired", here, expiresIn = (-1).hours)

        val kept = FilterActiveHazardsUseCase(clock).execute(listOf(live, expired))

        assertEquals(listOf("live"), kept.map { it.id })
    }

    @Test
    fun `only hazards inside the alert radius are returned`() {
        val rider = Coordinates(-70.65, -33.45)
        // 0.0003 degrees of latitude is about 33 m; 0.001 is about 111 m.
        val near = hazard("near", Coordinates(-70.65, -33.4503))
        val far = hazard("far", Coordinates(-70.65, -33.451))

        val inRange = HazardProximityUseCase(alertRadiusMeters = 50.0).findHazardsInRange(rider, listOf(near, far))

        assertEquals(listOf("near"), inRange.map { it.id })
    }

    @Test
    fun `the distance is the great-circle distance`() {
        val rider = Coordinates(0.0, 0.0)
        // One degree of longitude on the equator is 111.19 km with a 6371 km Earth.
        val oneDegreeEast = hazard("east", Coordinates(1.0, 0.0))

        assertEquals(
            listOf("east"),
            HazardProximityUseCase(alertRadiusMeters = 111_200.0).findHazardsInRange(rider, listOf(oneDegreeEast)).map { it.id },
        )
        assertEquals(
            emptyList<String>(),
            HazardProximityUseCase(alertRadiusMeters = 111_100.0).findHazardsInRange(rider, listOf(oneDegreeEast)).map { it.id },
        )
    }
}
