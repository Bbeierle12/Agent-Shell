import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { NoteCard } from './NoteCard'

describe('NoteCard', () => {
  it('renders a textarea with the given content', () => {
    render(<NoteCard content="hello world" onChange={() => {}} />)
    expect(screen.getByRole('textbox')).toHaveValue('hello world')
  })

  it('calls onChange when the user types', async () => {
    const onChange = vi.fn()
    render(<NoteCard content="" onChange={onChange} />)

    await userEvent.type(screen.getByRole('textbox'), 'a')
    expect(onChange).toHaveBeenCalledWith('a')
  })
})
