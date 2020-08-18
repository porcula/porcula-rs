# porcula-rs
Локальный поиск по электронной библиотеке

Реализация на Rust

## Цели
  * индексация FB2-книг внутри zip-файлов (например архивы librusec,flibusta)
  * полнотекстовый поиск по названию/автору/аннотации/тексту книги
  * примитивный веб-интерфейс, цель - работа на электронной читалке типа Onyx/PocketBook и ПК
  * просмотр книги в браузере как HTML
  * отсутствие runtime-зависимостей


## Требования
  * ОС поддерживаемая Rust (Linux,Windows,MacOS,...)
  * для большой библиотеки **требуется 64-битная ОС**
  * место под базу - примерно 1/6 от объёма исходных файлов
  * для индексации желательно мощное железо (от 4 ядер, 16 ГБ памяти, SSD)
  * поиск в веб-приложении требует гораздо меньше ресурсов

## Документация для пользователя

https://github.com/porcula/porcula-rs/wiki

## Сборка
```
cargo build --release
```
В режиме debug скорость приложения на порядок ниже!
