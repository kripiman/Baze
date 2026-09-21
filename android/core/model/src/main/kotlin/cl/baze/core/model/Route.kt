// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.serialization.Serializable

@Serializable
data class RouteManeuver(
    val instruction: String,
    val distanceMeters: Double,
    val timeSeconds: Double,
    val location: Coordinates
)

@Serializable
data class Route(
    val distanceMeters: Double,
    val durationSeconds: Double,
    val ascentMeters: Double,
    val descentMeters: Double,
    val points: List<Coordinates>,
    val maneuvers: List<RouteManeuver>,
    val nearbyHazards: List<Hazard>
)

@Serializable
data class Account(
    val id: String,
    val token: String
)
