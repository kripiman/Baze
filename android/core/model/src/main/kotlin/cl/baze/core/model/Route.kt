// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class GeoJsonLineString(
    val type: String = "LineString",
    val coordinates: List<List<Double>>
)

@Serializable
data class RouteManeuver(
    val instruction: String,
    @SerialName("distance_meters")
    val distanceMeters: Double,
    @SerialName("time_seconds")
    val timeSeconds: Double,
    val location: Coordinates
)

@Serializable
data class Route(
    @SerialName("distance_meters")
    val distanceMeters: Double,
    @SerialName("duration_seconds")
    val durationSeconds: Double,
    @SerialName("ascent_meters")
    val ascentMeters: Double,
    @SerialName("descent_meters")
    val descentMeters: Double,
    val geometry: GeoJsonLineString,
    val maneuvers: List<RouteManeuver>,
    @SerialName("nearby_hazards")
    val nearbyHazards: List<Hazard>
)

@Serializable
data class Account(
    @SerialName("account_id")
    val id: String,
    val token: String
)
