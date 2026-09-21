// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.location

import android.annotation.SuppressLint
import android.content.Context
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import cl.baze.core.model.Coordinates
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow

/**
 * Implementación 100% libre basada en el LocationManager nativo del sistema operativo Android.
 * No utiliza Google Play Services (FusedLocationProviderClient), permitiendo compilación
 * compatible con F-Droid y respeto a la licencia AGPLv3.
 */
class AndroidLocationManagerProvider(
    private val context: Context
) : LocationProvider {

    private val locationManager by lazy {
        context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
    }

    @SuppressLint("MissingPermission")
    override fun observeLocation(): Flow<Coordinates> = callbackFlow {
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) {
                trySend(
                    Coordinates(
                        longitude = location.longitude,
                        latitude = location.latitude,
                        altitudeMeters = if (location.hasAltitude()) location.altitude else null
                    )
                )
            }

            @Deprecated("Deprecated in Java")
            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) = Unit
            override fun onProviderEnabled(provider: String) = Unit
            override fun onProviderDisabled(provider: String) = Unit
        }

        val provider = when {
            locationManager.isProviderEnabled(LocationManager.GPS_PROVIDER) -> LocationManager.GPS_PROVIDER
            locationManager.isProviderEnabled(LocationManager.NETWORK_PROVIDER) -> LocationManager.NETWORK_PROVIDER
            else -> LocationManager.PASSIVE_PROVIDER
        }

        // TODO(verify): Ajustar minTimeMs y minDistanceM para optimizar ahorro de batería en trayectos largos
        locationManager.requestLocationUpdates(
            provider,
            1000L, // 1 segundo
            2.0f,  // 2 metros
            listener,
            context.mainLooper
        )

        awaitClose {
            locationManager.removeUpdates(listener)
        }
    }

    @SuppressLint("MissingPermission")
    override suspend fun getLastKnownLocation(): Coordinates? {
        val loc = locationManager.getLastKnownLocation(LocationManager.GPS_PROVIDER)
            ?: locationManager.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)

        return loc?.let {
            Coordinates(
                longitude = it.longitude,
                latitude = it.latitude,
                altitudeMeters = if (it.hasAltitude()) it.altitude else null
            )
        }
    }
}
