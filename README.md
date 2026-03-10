# 3cat Downloader (or 3xl/sx3)

_(English)_ This is a downloader for TV shows from 3cat.cat, the Catalan national TV channel. For the rest of the README, the instructions will be in Catalan. This script was done in an afternoon to download Dragon Ball for my son 🐉

Aquest és un script per descarregar les sèries de 3cat.cat (o d'altres plataformes de la televisió catalana com Súper3 o 3xl).

## Ús

Tens dues opcions per utilitzar aquest script: pots clonar el repositori i executar el script manualment (necessitaràs tenir Rust instal·lat), o bé descarregar directament el binari.

### Executar el script manualment

Clona aquest repositori i executa el script amb la següent comanda:

```bash
cargo run -- --tv-show-slug bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Executar el binari descarregat

Descarrega el binari des de [releases](https://github.com/mcamara/3cat_show_downloader/releases) i executa la següent comanda:

```bash
./cat_show_downloader --tv-show-slug bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Opcions

| Opció | Curt | Descripció | Per defecte |
|---|---|---|---|
| `--tv-show-slug` | `-t` | Slug de la sèrie (veure més avall) | *obligatori* |
| `--directory` | `-d` | Directori on desar els capítols | *obligatori* |
| `--start-from-episode` | `-s` | Número de capítol des del qual començar | `1` |
| `--concurrent-downloads` | `-c` | Número de capítols a descarregar alhora (1-10) | `2` |
| `--skip-subtitles` | | No descarregar els subtítols | `false` |
| `--fix-existing-subtitles` | `-f` | Netejar els fitxers de subtítols (.vtt) ja descarregats al directori | `false` |
| `--embed-existing-subtitles` | | Netejar i incrustar els subtítols als vídeos ja descarregats (requereix ffmpeg) | `false` |

Per exemple, per descarregar 4 capítols alhora:

```bash
./cat_show_downloader -t bola-de-drac -d ~/Downloads/bola-de-drac/ -c 4
```

Per descarregar sense subtítols:

```bash
./cat_show_downloader -t bola-de-drac -d ~/Downloads/bola-de-drac/ --skip-subtitles
```

Per netejar els subtítols ja descarregats (elimina les capçaleres `Region:` no estàndard i els atributs `region:rN` de les línies de temps):

```bash
./cat_show_downloader -t bola-de-drac -d ~/Downloads/bola-de-drac/ -f
```

Per netejar i incrustar els subtítols existents directament als fitxers de vídeo (requereix ffmpeg instal·lat):

```bash
./cat_show_downloader -t bola-de-drac -d ~/Downloads/bola-de-drac/ --embed-existing-subtitles
```

### Integració amb ffmpeg

Si tens [ffmpeg](https://ffmpeg.org/) instal·lat i accessible al PATH del sistema, els subtítols s'incrustaran automàticament als fitxers de vídeo durant la descàrrega. Els subtítols VTT es converteixen a format ASS (Advanced SubStation Alpha) per preservar l'estil original (colors, fons, etc.) i s'incrusten en un fitxer `.mkv` (Matroska) en lloc de `.mp4`. Els fitxers `.vtt` s'eliminen automàticament un cop incrustats.

Si ffmpeg no està instal·lat, els subtítols es descarregaran com a fitxers `.vtt` separats i el vídeo es mantindrà com a `.mp4` (el comportament original).

L'opció `--embed-existing-subtitles` permet incrustar els subtítols als vídeos que ja s'han descarregat prèviament. Aquesta opció també neteja els subtítols (igual que `--fix-existing-subtitles`) abans d'incrustar-los. Un cop incrustats, els fitxers `.vtt` i `.mp4` originals s'eliminen i es genera un fitxer `.mkv`. Aquesta opció requereix que ffmpeg estigui instal·lat.

### Com trobar el "slug" de la sèrie?

El "slug" és el nom de la sèrie en minúscules i sense espais. Si tens la URL de la pàgina de la sèrie, el "slug" és la part que apareix després de `/3cat/` i abans del següent `/` (si és de 3cat.cat). En el cas de Súper3 o 3xl, el "slug" apareix després de `/tc3/sx3/` i abans del següent `/`.

Per exemple:

- La URL de la sèrie "Bola de Drac" és https://www.3cat.cat/3cat/bola-de-drac/, el seu "slug" és `bola-de-drac`.
- Si fos una sèrie de Súper3, com per exemple https://www.3cat.cat/tv3/sx3/kuroko-basquet/, el "slug" seria `kuroko-basquet`.

### Problemes coneguts (i que probablement no solucionaré mai, per ser sincer)

- Després de descarregar alguns capítols, pot aparèixer un problema de xarxa, ja que TV3 "et fa fora". En aquest cas, prova d'executar el script novament després d'una estona. El script no descarregarà capítols ja descarregats (i si saps el número de capítol on ha parat, pots indicar-ho amb l'opció `--start-from-episode`).
- Si no estàs a Catalunya o Andorra, algunes sèries poden no estar disponibles. Això es pot solucionar fàcilment amb una VPN.

Si tens algun problema, pots crear una issue a [GitHub](https://github.com/mcamara/3cat_show_downloader/issues), i l'analitzaré quan tingui temps.

### Coses que potser millori en algun moment

- Testejar/Mockejar l'API de TV3 per a més fiabilitat.

### Nota personal

Aquest script el vaig fer en una tarda per poder descarregar Bola de Drac per al meu fill. Espero que et sigui útil i si vols ajudar a millorar-ho, no dubtis a crear un PR i enviar les teves millores.
