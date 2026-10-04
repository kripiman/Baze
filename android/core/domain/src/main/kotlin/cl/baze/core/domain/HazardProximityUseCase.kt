// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.domain

import cl.baze.core.model.Coordinates
import cl.baze.core.model.Hazard
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Caso de uso en Kotlin puro para determinar la proximidad del ciclista
 * a peligros viales conocidos sin depender de librerías de Android ni de java.*.
 */
class HazardProximityUseCase(
    private val alertRadiusMeters: Double = 50.0
) {
    /**
     * Retorna los peligros que se encuentran a menos de alertRadiusMeters del punto actual.
     */
    fun findHazardsInRange(
        currentLocation: Coordinates,
        hazards: List<Hazard>
    ): List<Hazard> {
        // TODO(verify): Implementar verificación con proyección a lo largo de la polilínea de la ruta
        return hazards.filter { hazard ->
            calculateHaversineDistanceMeters(currentLocation, hazard.location) <= alertRadiusMeters
        }
    }

    private fun calculateHaversineDistanceMeters(c1: Coordinates, c2: Coordinates): Double {
        val earthRadius = 6371000.0 // metros
        val dLat = (c2.latitude - c1.latitude).toRadians()
        val dLon = (c2.longitude - c1.longitude).toRadians()
        val lat1 = c1.latitude.toRadians()
        val lat2 = c2.latitude.toRadians()

        val a = sin(dLat / 2) * sin(dLat / 2) +
                cos(lat1) * cos(lat2) *
                sin(dLon / 2) * sin(dLon / 2)
        val c = 2 * atan2(sqrt(a), sqrt(1 - a))
        return earthRadius * c
    }

    private fun Double.toRadians(): Double = this * PI / 180.0
}
