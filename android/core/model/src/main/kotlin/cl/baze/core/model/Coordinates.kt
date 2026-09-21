// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.serialization.Serializable

@Serializable
data class Coordinates(
    val longitude: Double,
    val latitude: Double,
    val altitudeMeters: Double? = null
)

@Serializable
data class BoundingBox(
    val minLongitude: Double,
    val minLatitude: Double,
    val maxLongitude: Double,
    val maxLatitude: Double
) {
    fun contains(coordinates: Coordinates): Boolean {
        return coordinates.longitude in minLongitude..maxLongitude &&
                coordinates.latitude in minLatitude..maxLatitude
    }
}
