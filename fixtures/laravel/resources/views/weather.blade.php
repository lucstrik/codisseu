@extends('layouts.app')
@section('content')
    @foreach ($forecasts as $forecast)
        <span>{{ $forecast }}</span>
    @endforeach
@endsection
