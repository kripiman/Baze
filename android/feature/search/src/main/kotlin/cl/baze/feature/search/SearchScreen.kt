// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.feature.search

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import cl.baze.core.model.Coordinates

/**
 * Pantalla y campo de búsqueda de direcciones.
 * Consulta exclusivamente el endpoint del backend /api/v1/geocoding/search (proxy hacia Photon).
 */
@Composable
fun SearchBar(
    modifier: Modifier = Modifier,
    onLocationSelected: (Coordinates, String) -> Unit = { _, _ -> }
) {
    var query by remember { mutableStateOf("") }

    Column(modifier = modifier.fillMaxWidth().padding(16.dp)) {
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            label = { Text("Buscar dirección o destino...") },
            modifier = Modifier.fillMaxWidth()
        )
        // TODO(verify): Implementar lista desplegable de resultados conectada a BazeApiClient
    }
}
