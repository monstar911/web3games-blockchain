#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{
	dispatch::{DispatchError, DispatchResult},
	ensure,
	traits::{Currency, Get, ReservableCurrency},
	PalletId, BoundedVec,
};
use primitives::Balance;
use sp_runtime::{
	traits::{AtLeast32BitUnsigned, One, CheckedAdd},
	RuntimeDebug,
};
use sp_std::{convert::TryInto, prelude::*};

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

type BalanceOf<T> =
	<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, MaxEncodedLen)]
pub struct Token<AccountId, BoundedString> {
	owner: AccountId,
	name: BoundedString,
	symbol: BoundedString,
	decimals: u8,
	total_supply: Balance,
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{dispatch::DispatchResult, pallet_prelude::*};
	use frame_system::pallet_prelude::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type PalletId: Get<PalletId>;

		/// Identifier for the class of token.
		type FungibleTokenId: Member + Parameter + AtLeast32BitUnsigned + Default + Copy + MaxEncodedLen;

		/// The maximum length of a name or symbol stored on-chain.
		#[pallet::constant]
		type StringLimit: Get<u32>;

		/// The minimum balance to create token
		#[pallet::constant]
		type CreateTokenDeposit: Get<BalanceOf<Self>>;

		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	pub(super) type Tokens<T: Config> =
		StorageMap<_, Blake2_128Concat, T::FungibleTokenId, Token<T::AccountId, BoundedVec<u8, T::StringLimit>>>;

	#[pallet::storage]
	#[pallet::getter(fn next_token_id)]
	pub(super) type NextTokenId<T: Config> = StorageValue<_, T::FungibleTokenId, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn balance_of)]
	pub(super) type Balances<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::FungibleTokenId,
		Blake2_128Concat,
		T::AccountId,
		Balance,
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn allowances)]
	pub(super) type Allowances<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::FungibleTokenId,
		Blake2_128Concat,
		// (owner, operator)
		(T::AccountId, T::AccountId),
		Balance,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::metadata(T::AccountId = "AccountId")]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		TokenCreated(T::FungibleTokenId, T::AccountId),
		Transfer(T::FungibleTokenId, T::AccountId, T::AccountId, Balance),
		Approval(T::FungibleTokenId, T::AccountId, T::AccountId, Balance),
	}

	#[pallet::error]
	pub enum Error<T> {
		Unknown,
		NoAvailableTokenId,
		NumOverflow,
		NoPermission,
		NotOwner,
		InvalidId,
		AmountExceedAllowance,
		BadMetadata,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(10_000)]
		pub fn create_token(
			origin: OriginFor<T>,
			name: Vec<u8>,
			symbol: Vec<u8>,
			decimals: u8,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_create_token(&who, name, symbol, decimals)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn approve(
			origin: OriginFor<T>,
			id: T::FungibleTokenId,
			spender: T::AccountId,
			amount: Balance,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
	
			Allowances::<T>::try_mutate(id, (&who, &spender), |allowance| -> DispatchResult {
				*allowance = allowance
					.checked_add(amount)
					.ok_or(Error::<T>::NumOverflow)?;
				Ok(())
			})?;
	
			Self::deposit_event(Event::Transfer(
				id,
				who.clone(),
				spender.clone(),
				amount,
			));

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn transfer(
			origin: OriginFor<T>,
			id: T::FungibleTokenId,
			recipient: T::AccountId,
			amount: Balance,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_transfer(id, &who, &recipient, amount)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn transfer_from(
			origin: OriginFor<T>,
			id: T::FungibleTokenId,
			sender: T::AccountId,
			recipient: T::AccountId,
			amount: Balance,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
	
			Allowances::<T>::try_mutate(id, (&sender, &who), |allowance| -> DispatchResult {
				*allowance = allowance
					.checked_sub(amount)
					.ok_or(Error::<T>::NumOverflow)?;
				Ok(())
			})?;

			Self::do_transfer(id, &sender, &recipient, amount)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn mint(
			origin: OriginFor<T>,
			id: T::FungibleTokenId,
			account: T::AccountId,
			amount: Balance,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::maybe_check_permission(id, &who)?;

			Self::do_mint(id, &account, amount)?;

			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn burn(
			origin: OriginFor<T>,
			id: T::FungibleTokenId,
			amount: Balance,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_burn(id, &who, amount)?;

			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	pub fn exists(id: T::FungibleTokenId) -> bool {
		Tokens::<T>::contains_key(id)
	}

	pub fn total_supply(id: T::FungibleTokenId) -> Result<Balance, DispatchError> {
		let token = Tokens::<T>::get(id).ok_or(Error::<T>::InvalidId)?;
		Ok(token.total_supply)
	}

	pub fn do_create_token(
		who: &T::AccountId,
		name: Vec<u8>,
		symbol: Vec<u8>,
		decimals: u8,
	) -> Result<T::FungibleTokenId, DispatchError> {
		let deposit = T::CreateTokenDeposit::get();
		T::Currency::reserve(&who, deposit.clone())?;

		let bounded_name: BoundedVec<u8, T::StringLimit> =
			name.clone().try_into().map_err(|_| Error::<T>::BadMetadata)?;
		let bounded_symbol: BoundedVec<u8, T::StringLimit> =
			symbol.clone().try_into().map_err(|_| Error::<T>::BadMetadata)?;

		let id = NextTokenId::<T>::try_mutate(|id| -> Result<T::FungibleTokenId, DispatchError> {
			let current_id = *id;
			*id = id.checked_add(&One::one()).ok_or(Error::<T>::NoAvailableTokenId)?;
			Ok(current_id)
		})?;

		let token = Token {
			owner: who.clone(),
			name: bounded_name,
			symbol: bounded_symbol,
			decimals,
			total_supply: Balance::default(),
		};

		Tokens::<T>::insert(id, token);

		Self::deposit_event(Event::TokenCreated(id, who.clone()));

		Ok(id)
	}

	pub fn do_transfer(
		id: T::FungibleTokenId,
		sender: &T::AccountId,
		recipient: &T::AccountId,
		amount: Balance,
	) -> DispatchResult {
		Self::decrease_balance(id, sender, amount)?;
		Self::increase_balance(id, recipient, amount)?;

		Self::deposit_event(Event::Transfer(
			id,
			sender.clone(),
			recipient.clone(),
			amount,
		));

		Ok(())
	}

	pub fn do_mint(
		id: T::FungibleTokenId,
		account: &T::AccountId,
		amount: Balance,
	) -> DispatchResult {
		Tokens::<T>::try_mutate(id, |maybe_token| -> DispatchResult {
			let token = maybe_token.as_mut().ok_or(Error::<T>::Unknown)?;

			Self::increase_balance(id, account, amount)?;

			let new_total_supply = token.total_supply.saturating_add(amount);
			token.total_supply = new_total_supply;
			Ok(())
		})?;

		Self::deposit_event(Event::Transfer(
			id,
			T::AccountId::default(),
			account.clone(),
			amount,
		));

		Ok(())
	}

	pub fn do_burn(
		id: T::FungibleTokenId,
		account: &T::AccountId,
		amount: Balance,
	) -> DispatchResult {

		Tokens::<T>::try_mutate(id, |maybe_token| -> DispatchResult {
			let token = maybe_token.as_mut().ok_or(Error::<T>::Unknown)?;

			Self::decrease_balance(id, account, amount)?;

			let new_total_supply = token.total_supply.saturating_sub(amount);
			token.total_supply = new_total_supply;
			Ok(())
		})?;

		Self::deposit_event(Event::Transfer(
			id,
			account.clone(),
			T::AccountId::default(),
			amount,
		));

		Ok(())
	}

	fn increase_balance(
		id: T::FungibleTokenId,
		to: &T::AccountId,
		amount: Balance,
	) -> DispatchResult {
		Balances::<T>::try_mutate(id, to, |balance| -> DispatchResult {
			*balance = balance.checked_add(amount).ok_or(Error::<T>::NumOverflow)?;
			Ok(())
		})?;

		Ok(())
	}

	fn decrease_balance(
		id: T::FungibleTokenId,
		from: &T::AccountId,
		amount: Balance,
	) -> DispatchResult {
		Balances::<T>::try_mutate(id, from, |balance| -> DispatchResult {
			*balance = balance.checked_sub(amount).ok_or(Error::<T>::NumOverflow)?;
			Ok(())
		})?;

		Ok(())
	}

	fn maybe_check_permission(id: T::FungibleTokenId, who: &T::AccountId) -> DispatchResult {
		let token = Tokens::<T>::get(id).ok_or(Error::<T>::InvalidId)?;
		ensure!(*who == token.owner, Error::<T>::NoPermission);

		Ok(())
	}
}
